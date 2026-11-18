# NixOS VM test: opencode as the engine (ADR-058) — the real opencode (upstream's release, pinned:
# opencode/package.nix) run by the agent's service and driven through the Agent Client Protocol, its model a
# stand-in reached through TorqOS's model proxy. The claims, each a subtest that fails if the claim breaks:
# - `torq agent engine opencode`: a question asked with `torq ask` is answered with the model's words, through the
#   proxy, recorded as alice's agent's; the model's service is told the placeholder key and nothing of a sign-in;
#   a second question, after the service is started again, is asked in the same conversation (opencode is given
#   it back, and the model is sent the first question with the second);
# - TorqOS's settings for it are in its environment (its config, its switches, a password for its own server),
#   never in /etc/opencode, and the password is on no command line and readable by no other account;
# - opencode's own HTTP server is on alice's agent's own port, and no other account reaches it — bob, carol, root,
#   and a program alice runs as her agent at a terminal — nor does anything of her agent's reach another account's
#   program listening on that port in its place;
# - nothing leaves the machine but the model's requests: the stand-in internet sees no other, and each other site
#   opencode asks for is refused by the proxy and recorded;
# - with no API key, the model refusing it (401), the person reads that in plain words;
# - Forget deletes opencode's own record of the conversation (`opencode session delete`);
# - `torq agent run -- opencode` runs it as the agent, with TorqOS's settings.
{ pkgs, self }:

let
  hash = "$6$torqsocketsalt$Z7RLey3l5vKGfukhfuZYYM/EjWNo7QM8.65r88XmZ9z3/jBCY.NAnH7RwYpiLoE2fedxUd/I7UFcKDX1KEXDg/";
  # An authority for this test alone, and a certificate it signed for the model's service (tests/model-proxy.nix).
  pki = pkgs.runCommand "torq-opencode-test-pki" { nativeBuildInputs = [ pkgs.openssl ]; } ''
    mkdir $out && cd $out
    openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -keyout ca.key -out ca.pem -days 3650 \
      -subj "/CN=TorqOS test authority" -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign"
    openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -keyout site.key -out site.csr -subj "/CN=the internet"
    printf 'subjectAltName=DNS:api.anthropic.com\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n' > ext
    openssl x509 -req -in site.csr -CA ca.pem -CAkey ca.key -CAcreateserial -out site.pem -days 3650 -extfile ext
  '';
  # The model's service, standing in: every request written down (its path, its host, the names of its headers and
  # the key it carried, its body); a question that asks for "no key" refused as Anthropic refuses a missing key
  # (401), any other answered with a short stream, as Anthropic's (the events opencode 1.15.10 was seen to take).
  # Anything that is not a request for the model is written down too, and answered 404.
  model = pkgs.writeText "model.py" ''
    import http.server, json, ssl, socket

    class H(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"
        def log_message(self, *a):
            pass
        def note(self, body):
            with open("/tmp/model-requests", "a") as f:
                f.write(json.dumps({"method": self.command, "path": self.path, "host": self.headers.get("host"), "headers": sorted(k.lower() for k in self.headers.keys()), "key": self.headers.get("x-api-key"), "body": body}) + "\n")
        def do_GET(self):
            self.note("")
            self.send_response(404)
            self.send_header("content-length", "0")
            self.end_headers()
        def do_POST(self):
            body = self.rfile.read(int(self.headers.get("content-length", 0))).decode()
            self.note(body)
            if not self.path.startswith("/v1/messages"):
                self.send_response(404)
                self.send_header("content-length", "0")
                self.end_headers()
                return
            if "no key please" in body:
                b = json.dumps({"type": "error", "error": {"type": "authentication_error", "message": "x-api-key header is required"}}).encode()
                self.send_response(401)
                self.send_header("content-type", "application/json")
                self.send_header("content-length", str(len(b)))
                self.end_headers()
                self.wfile.write(b)
                return
            events = [
                ("message_start", {"type": "message_start", "message": {"id": "msg_1", "type": "message", "role": "assistant", "model": "claude-test", "content": [], "stop_reason": None, "stop_sequence": None, "usage": {"input_tokens": 10, "output_tokens": 1}}}),
                ("content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
                ("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "hello from the stand-in model"}}),
                ("content_block_stop", {"type": "content_block_stop", "index": 0}),
                ("message_delta", {"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": None}, "usage": {"output_tokens": 6}}),
                ("message_stop", {"type": "message_stop"}),
            ]
            self.send_response(200)
            self.send_header("content-type", "text/event-stream")
            self.send_header("transfer-encoding", "chunked")
            self.end_headers()
            for name, e in events:
                data = ("event: %s\ndata: %s\n\n" % (name, json.dumps(e))).encode()
                self.wfile.write(b"%x\r\n%s\r\n" % (len(data), data))
                self.wfile.flush()
            self.wfile.write(b"0\r\n\r\n")

    class V6(http.server.ThreadingHTTPServer):
        address_family = socket.AF_INET6

    srv = V6(("::", 443), H)
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain("${pki}/site.pem", "${pki}/site.key")
    srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
    srv.serve_forever()
  '';
  # A program that listens on a port and writes down whatever reaches it (`listen <port> <file>`).
  listen = pkgs.writeText "listen.py" ''
    import socket, sys
    s = socket.socket()
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(("127.0.0.1", int(sys.argv[1])))
    s.listen()
    print("listening", flush=True)
    while True:
        c, _ = s.accept()
        c.settimeout(3)
        try:
            data = c.recv(4096)
        except OSError:
            data = b""
        with open(sys.argv[2], "ab") as f:
            f.write(b"reached: " + data[:40] + b"\n")
        c.close()
  '';
in
pkgs.testers.runNixOSTest {
  name = "torq-agent-opencode";
  # `torq unfree` sets nixpkgs.config (modules/unfree.nix), so the node builds its own package set.
  node.pkgsReadOnly = false;

  nodes.internet = { pkgs, ... }: {
    virtualisation.vlans = [ 1 ];
    networking.firewall.allowedTCPPorts = [ 443 ];
    systemd.services.model = { wantedBy = [ "multi-user.target" ]; serviceConfig.ExecStart = "${pkgs.python3}/bin/python3 ${model}"; };
  };

  nodes.machine = { pkgs, lib, ... }: {
    imports = [ self.nixosModules.default ];
    virtualisation.vlans = [ 1 ];
    virtualisation.memorySize = 2048;
    torq.user = "alice";
    torq.desktop.enable = false;
    users.users.root.hashedPasswordFile = lib.mkForce null;
    users.users.alice = { hashedPasswordFile = lib.mkForce null; hashedPassword = hash; };
    users.users.bob = { isNormalUser = true; hashedPasswordFile = lib.mkForce null; hashedPassword = hash; };
    users.users.carol = { isNormalUser = true; hashedPasswordFile = lib.mkForce null; hashedPassword = hash; };
    torq.agent.people = [ "alice" "bob" ];
    torq.broker.rebuildCommand = "true";
    nix.settings.min-free = 0;
    # The engine: opencode, the real one, installed for the agents.
    torq.agent.tools = [ "opencode" ];
    torq.agent.engine = "opencode";
    # The model's service as this machine finds it: the test's authority trusted, its name by the hosts file.
    security.pki.certificateFiles = [ "${pki}/ca.pem" ];
    networking.hosts."192.168.1.1" = [ "api.anthropic.com" ];
    environment.systemPackages = [ pkgs.curl pkgs.jq pkgs.python3 pkgs.socat pkgs.procps pkgs.iproute2 pkgs.nftables pkgs.sqlite ];
  };

  testScript = ''
    import json, shlex
    start_all()
    internet.wait_for_unit("model.service")
    internet.wait_for_open_port(443)
    machine.wait_for_unit("multi-user.target")
    machine.wait_for_unit("torq-broker.socket")
    machine.wait_for_unit("torq-model-proxy.service")
    machine.wait_for_unit("torq-agent-network.service")
    repo = "/var/lib/torq/config"
    machine.succeed(f"mkdir -p {repo} && echo '{{}}' > {repo}/desktop.json && touch {repo}/flake.nix && torq-broker init")
    unit = "torq-agent@alice.service"
    home = "/var/lib/torq-agent-alice"
    record = "/var/lib/torq-model-proxy/alice"
    engine = "pgrep -u torq-agent-alice -f 'opencode acp'"

    def as_user(user, cmd, ok=True):
        code, out = machine.execute(f"su - {user} -c {shlex.quote(cmd)} 2>&1")
        assert (code == 0) == ok, f"{user}: {cmd}: exit {code}, expected {'success' if ok else 'failure'}:\n{out}"
        return out

    def requests():
        return [json.loads(l) for l in internet.succeed("cat /tmp/model-requests 2>/dev/null || true").splitlines()]

    def lines():
        return [json.loads(l) for l in machine.succeed(f"cat {record}/*.jsonl 2>/dev/null || true").splitlines()]

    with subtest("opencode is a tool and an engine of the tree, installed in the agent's own profile"):
        values = json.loads(machine.succeed("jq -c '[.settings[] | select(.loc == [\"torq\", \"agent\", \"engine\"]) | .type.elem.values]' /etc/torq/nixos-schema.json"))
        assert values == [["claude-code", "opencode"]], values
        assert json.loads(machine.succeed("cat /etc/torq/agent-tools.json")) == {"tools": ["opencode"], "engine": "opencode", "plainCommand": False}
        assert machine.succeed("/etc/profiles/per-user/torq-agent-alice/bin/opencode --version").strip() == "1.15.10"
        machine.fail("test -e /etc/opencode")
        machine.fail("su - carol -c 'command -v opencode'")
        # The fence's rules for opencode's own port: alice's agent's is 7810, bob's 7811.
        table = machine.succeed("nft list table inet torq_agents")
        for want in ['tcp dport 7810 ct state new drop', 'tcp sport 7811 drop', 'tcp dport 7810 accept', 'tcp sport 7810 ct direction reply accept']:
            assert want in table, table

    with subtest("torq ask: opencode answers, through the model proxy, recorded as alice's agent's"):
        out = as_user("alice", "torq ask 'say hello'")
        assert "hello from the stand-in model" in out, out
        asked = [r for r in requests() if r["path"].startswith("/v1/messages")]
        assert asked and all(r["host"] == "api.anthropic.com" and r["key"] == "not-a-key:the-torqos-model-proxy-adds-it" for r in asked), asked
        assert any("say hello" in r["body"] for r in asked), asked
        assert all("authorization" not in r["headers"] for r in asked), "no sign-in of any kind passed"
        mine = [l for l in lines() if l["kind"] == "request"]
        assert mine and all((l["agent"], l["person"], l["status"]) == ("torq-agent-alice", "alice", 200) for l in mine), mine
        assert "not-a-key" not in machine.succeed(f"cat {record}/*.jsonl"), "no key in the record, not even the placeholder"
        machine.succeed(engine)
        machine.succeed("ss -ltnH 'sport = :7810' | grep -q 127.0.0.1:7810")

    with subtest("its settings are in its environment, with a password for its own server no other account reads"):
        pid = machine.succeed(engine).split()[0]
        env = machine.succeed(f"tr '\\0' '\\n' < /proc/{pid}/environ")
        config = json.loads([l for l in env.splitlines() if l.startswith("OPENCODE_CONFIG_CONTENT=")][0].split("=", 1)[1])
        assert config["provider"]["anthropic"]["options"]["baseURL"] == "http://127.0.0.1:7801/v1", config
        assert config["mcp"]["torqos"]["command"] == ["/run/current-system/sw/bin/torq-mcp"] and config["share"] == "disabled", config
        for switch in ["OPENCODE_DISABLE_MODELS_FETCH=1", "OPENCODE_PURE=1", "OPENCODE_DISABLE_SHARE=1"]:
            assert switch in env.splitlines(), env
        password = [l for l in env.splitlines() if l.startswith("OPENCODE_SERVER_PASSWORD=")][0].split("=", 1)[1]
        assert len(password) == 64, password
        cmdline = machine.succeed(f"tr '\\0' ' ' < /proc/{pid}/cmdline")
        assert password not in cmdline and "--port 7810 --hostname 127.0.0.1" in cmdline, cmdline
        for who in ["bob", "carol"]:
            as_user(who, f"cat /proc/{pid}/environ", ok=False)
        machine.fail(f"grep -rqs {password} /etc /run /tmp /var/lib/torq-agent-memory")

    with subtest("opencode's own server: no other account reaches it, nor a program alice runs as her agent"):
        reach = "curl -s --noproxy '*' --max-time 5 -o /dev/null -w '%{http_code}' http://127.0.0.1:7810/session"
        for who in ["bob", "carol"]:
            out = as_user(who, reach, ok=False)
            assert "401" not in out and "200" not in out, f"{who} reached the server: {out}"
        code, out = machine.execute(reach)
        assert code != 0 and out.strip() in ("000", ""), f"root reached the server: {code} {out}"
        out = as_user("alice", f"torq agent run -- sh -c {shlex.quote(reach)}", ok=False)
        assert "200" not in out and "401" not in out, out
        out = as_user("bob", f"torq agent run -- sh -c {shlex.quote(reach)}", ok=False)
        assert "200" not in out and "401" not in out, out
        # (The control: the same server answers its own service, the question above; and with the password, which
        # only that service has, it is the server that would decide, not the network.)

    with subtest("a conversation continued after the service starts again: opencode is given it back"):
        machine.succeed(f"systemctl restart {unit} || systemctl stop {unit}")
        machine.wait_until_fails(engine)
        before = len(requests())
        out = as_user("alice", "torq ask 'and again'")
        assert "hello from the stand-in model" in out, out
        later = [r for r in requests()[before:] if "and again" in r["body"]]
        assert later and "say hello" in later[0]["body"], "the earlier question went with the later one: the same conversation"

    with subtest("nothing of its own leaves the machine: only the model's requests arrive, every other site refused"):
        assert all(r["path"].startswith("/v1/messages") for r in requests()), requests()
        sites = [l for l in lines() if l["kind"] == "connect"]
        assert all(l["allowed"] is False for l in sites), sites
        print("sites opencode asked for, refused:", sorted(set(l["host"] for l in sites)))

    with subtest("a program alice's agent runs cannot reach another account's program on opencode's port"):
        machine.succeed(f"systemctl stop {unit}")
        machine.wait_until_fails(engine)
        machine.succeed("su - carol -c 'python3 ${listen} 7810 /tmp/carol-heard >/tmp/carol-listen 2>&1 &'")
        machine.wait_until_succeeds("grep -q listening /tmp/carol-listen")
        as_user("alice", "torq agent run -- sh -c 'echo SECRET | timeout 5 socat - TCP:127.0.0.1:7810'", ok=False)
        machine.succeed("su -s /bin/sh torq-agent-alice -c 'echo SECRET2 | timeout 5 socat - TCP:127.0.0.1:7810' || true")
        machine.fail("grep -q SECRET /tmp/carol-heard")
        # (The control: carol's program does listen there. The port is opencode's on this machine: nothing reaches
        # another account's program on it, whoever sends, so no person's own connection is the control either.)
        machine.succeed("ss -ltnH 'sport = :7810' | grep -q 127.0.0.1:7810")
        machine.succeed("pkill -u carol python3")

    with subtest("Forget deletes opencode's own record of the conversation"):
        out = as_user("alice", "torq ask 'say hello'")
        session = json.loads(machine.succeed("cat /var/lib/torq-agent-memory/alice/conversation.json"))["session"]
        assert session.startswith("ses_"), session
        db = f"{home}/.local/share/opencode/opencode.db"
        assert session in machine.succeed(f"sqlite3 -readonly {db} 'select id from session'"), "the control: opencode has it"
        out = as_user("alice", "torq ask --forget")
        assert "rror" not in out, out
        assert session not in machine.succeed(f"sqlite3 -readonly {db} 'select id from session'"), "forgotten, and opencode still has it"

    with subtest("torq agent run -- opencode: as the agent, with TorqOS's settings"):
        out = as_user("alice", "torq agent run -- sh -c 'id -un; opencode --version; echo \"$OPENCODE_CONFIG_CONTENT\" | jq -r .provider.anthropic.options.baseURL'")
        assert out.split()[-3:] == ["torq-agent-alice", "1.15.10", "http://127.0.0.1:7801/v1"], out

    # Last: the refused question stays in the conversation, which the stand-in would refuse from then on.
    with subtest("no API key (the model refuses it): said in plain words"):
        out = as_user("alice", "torq ask 'no key please'", ok=False)
        assert "opencode got no answer from its model. It has no API key yet" in out, out
        assert any(l["kind"] == "request" and l["status"] == 401 for l in lines()), lines()

  '';
}
