# NixOS VM test: the model proxy and the agents' closed network (ADR-057). The claims, each as a subtest that
# fails if the claim breaks:
# - an agent's account reaches nothing on the network directly (IPv4, IPv6, another port of this machine, UDP,
#   DNS by any of the machine's ways to look a name up), and a person's own account is not touched;
# - it reaches its model only through the proxy, and each request is recorded as that person's agent's, named
#   by the kernel, whatever the request says of itself; a credential passes and is never in the record;
# - a site not listed is refused, in words that name the setting, and recorded; a listed one works; the web
#   setting opens every site but neither this machine, nor its own network, nor the model past the record;
# - the record is read by its person and by no one else, and written by no one but the proxy;
# - Claude Code is told of the proxy by settings only an agent's account sees, in each of the three ways the
#   engine starts; and nothing runs as an agent while the fence is not up.
#
# The node `internet` stands in for it: the model's service and a few sites, by HTTPS with a certificate of
# an authority made for this test (which `machine` is told to trust, as a machine trusts the real ones), a
# plain web server, and something that listens where DNS does and writes down every name it is asked.
{ pkgs, self }:

let
  hash = "$6$torqsocketsalt$Z7RLey3l5vKGfukhfuZYYM/EjWNo7QM8.65r88XmZ9z3/jBCY.NAnH7RwYpiLoE2fedxUd/I7UFcKDX1KEXDg/";
  names = [ "api.anthropic.com" "listed.example" "unlisted.example" "web.example" "local.example" ];
  # An authority for this test alone, and a certificate it signed for the names `internet` answers as.
  pki = pkgs.runCommand "torq-model-proxy-test-pki" { nativeBuildInputs = [ pkgs.openssl ]; } ''
    mkdir $out && cd $out
    openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -keyout ca.key -out ca.pem -days 3650 \
      -subj "/CN=TorqOS test authority" -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign"
    openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -keyout site.key -out site.csr -subj "/CN=the internet"
    printf 'subjectAltName=${pkgs.lib.concatMapStringsSep "," (n: "DNS:${n}") names}\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n' > ext
    openssl x509 -req -in site.csr -CA ca.pem -CAkey ca.key -CAcreateserial -out site.pem -days 3650 -extfile ext
  '';
  # The internet's side: HTTPS for every name (the model's service answers a request for its messages with a
  # short stream; anything else says which name it was asked as), and what it was sent, without the credential:
  # only whether the credential that was sent arrived.
  sites = pkgs.writeText "sites.py" ''
    import http.server, json, ssl

    class H(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"
        def log_message(self, *a):
            pass
        def do_POST(self):
            body = self.rfile.read(int(self.headers.get("content-length", 0)))
            with open("/tmp/model-requests", "a") as f:
                f.write(json.dumps({"path": self.path, "host": self.headers.get("host"), "version": self.headers.get("anthropic-version"),
                                    "credential_arrived": self.headers.get("authorization") == "Bearer CREDENTIAL-FOR-THE-TEST", "body": body.decode()}) + "\n")
            events = [
                {"type": "message_start", "message": {"usage": {"input_tokens": 7, "output_tokens": 1}}},
                {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}},
                {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "hello from the model"}},
                {"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 5}},
            ]
            self.send_response(200)
            self.send_header("content-type", "text/event-stream")
            self.send_header("transfer-encoding", "chunked")
            self.end_headers()
            for e in events:
                data = ("event: %s\ndata: %s\n\n" % (e["type"], json.dumps(e))).encode()
                self.wfile.write(b"%x\r\n%s\r\n" % (len(data), data))
                self.wfile.flush()
            self.wfile.write(b"0\r\n\r\n")
        def do_GET(self):
            body = ("hello from " + self.headers.get("host", "?") + "\n").encode()
            self.send_response(200)
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    class V6(http.server.ThreadingHTTPServer):
        address_family = __import__("socket").AF_INET6

    srv = V6(("::", 443), H)
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain("${pki}/site.pem", "${pki}/site.key")
    srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
    srv.serve_forever()
  '';
  # Where DNS listens: every datagram's readable text written down (a name asked, or a test's own marker), and
  # "no such name" answered at once to what is a question.
  names-asked = pkgs.writeText "names-asked.py" ''
    import socket
    s = socket.socket(socket.AF_INET6, socket.SOCK_DGRAM)
    s.bind(("::", 53))
    while True:
        data, peer = s.recvfrom(4096)
        with open("/tmp/names-asked", "a") as f:
            f.write("".join(chr(b) if 32 < b < 127 else "." for b in data) + "\n")
        if len(data) > 12:
            s.sendto(data[:2] + b"\x81\x83" + data[4:6] + b"\x00\x00\x00\x00\x00\x00" + data[12:], peer)
  '';
  # Someone else trying to take the proxy's port: bind it (on 127.0.0.1, or on every address, with every option
  # that lets a second socket share a port), say whether it could, and with `hold`, keep it.
  squat = pkgs.writeText "squat.py" ''
    import socket, sys, time
    s = socket.socket()
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
    try:
        s.bind((sys.argv[1], 7801))
        s.listen()
    except OSError as e:
        print("refused:", e.strerror, flush=True)
        sys.exit(1)
    print("bound", flush=True)
    while len(sys.argv) > 2:
        c, _ = s.accept()
        c.sendall(b"HTTP/1.1 200 OK\r\ncontent-length: 8\r\n\r\nsquatted")
        c.close()
  '';
  # Claude Code's stand-in, as the agents' engine: it writes down what it was told and what it can reach, where
  # the test reads it, and ends. (`service` when the agent's service started it, `plain` at a terminal.)
  claude = pkgs.writeShellScriptBin "claude" ''
    case "$*" in *stream-json*) how=service ;; *) how=plain ;; esac
    {
      echo "base=$ANTHROPIC_BASE_URL"
      echo "proxy=$HTTPS_PROXY"
      echo "managed=$(${pkgs.jq}/bin/jq -c .env /etc/claude-code/managed-settings.json 2>&1)"
      echo "direct=$(${pkgs.curl}/bin/curl -s --noproxy '*' --max-time 4 http://192.168.1.1:8080/ >/dev/null 2>&1 && echo open || echo closed)"
      echo "model=$(${pkgs.curl}/bin/curl -s --max-time 20 "$ANTHROPIC_BASE_URL/v1/messages" -H 'x-claude-code-session-id: from-'$how -d '{"model":"m","messages":[{"role":"user","content":"asked by the engine, '$how'"}]}' | grep -c text_delta)"
      echo "name=$(${pkgs.getent}/bin/getent hosts "engine-$how.example" >/dev/null 2>&1 && echo found || echo none)"
    } > "$HOME/facts-$how" 2>&1
  '';
in
pkgs.testers.runNixOSTest {
  name = "torq-model-proxy";
  # `torq unfree` sets nixpkgs.config (modules/unfree.nix), so the node builds its own package set.
  node.pkgsReadOnly = false;

  nodes.internet = { pkgs, ... }: {
    virtualisation.vlans = [ 1 ];
    networking.interfaces.eth1.ipv4.addresses = [ { address = "203.0.113.10"; prefixLength = 24; } ];
    networking.interfaces.eth1.ipv6.addresses = [ { address = "fd00::10"; prefixLength = 64; } ];
    networking.firewall.allowedTCPPorts = [ 443 8080 ];
    networking.firewall.allowedUDPPorts = [ 53 ];
    systemd.services.sites = { wantedBy = [ "multi-user.target" ]; serviceConfig.ExecStart = "${pkgs.python3}/bin/python3 ${sites}"; };
    systemd.services.names-asked = { wantedBy = [ "multi-user.target" ]; serviceConfig.ExecStart = "${pkgs.python3}/bin/python3 ${names-asked}"; };
    systemd.services.plain = { wantedBy = [ "multi-user.target" ]; serviceConfig.ExecStart = "${pkgs.python3}/bin/python3 -m http.server 8080 --bind :: --directory /var/empty"; };
  };

  nodes.machine = { pkgs, lib, ... }: {
    imports = [ self.nixosModules.default ];
    virtualisation.vlans = [ 1 ];
    torq.user = "alice";
    torq.desktop.enable = false;
    users.users.root.hashedPasswordFile = lib.mkForce null;
    users.users.alice = { hashedPasswordFile = lib.mkForce null; hashedPassword = hash; };
    users.users.bob = { isNormalUser = true; extraGroups = [ "wheel" ]; hashedPasswordFile = lib.mkForce null; hashedPassword = hash; };
    # A person with no agent.
    users.users.carol = { isNormalUser = true; hashedPasswordFile = lib.mkForce null; hashedPassword = hash; };
    torq.agent.people = [ "alice" "bob" ];
    torq.unfree.packages = [ "claude-code" ];
    torq.agent.tools = [ "claude-code" ];
    torq.agent.engine = "claude-code";
    torq.agent.claudeCode = claude;
    torq.broker.rebuildCommand = "true";
    # What the owner set: one site. And the same machine with the web opened, to switch to.
    torq.agent.network.hosts = [ "listed.example" ];
    specialisation.web.configuration.torq.agent.network.web = true;
    # The internet as this machine finds it: the test's authority among those it trusts, the names by its hosts
    # file (the model's service, a listed site and an unlisted one on the test's own network; one site at an
    # address that is not a private network's; one name for this machine itself), and DNS at `internet`.
    security.pki.certificateFiles = [ "${pki}/ca.pem" ];
    networking.hosts = {
      "192.168.1.1" = [ "api.anthropic.com" "listed.example" "unlisted.example" ];
      "203.0.113.10" = [ "web.example" ];
      "127.0.0.1" = [ "local.example" ];
    };
    networking.nameservers = [ "192.168.1.1" ];
    networking.interfaces.eth1.ipv4.addresses = [ { address = "203.0.113.11"; prefixLength = 24; } ];
    networking.interfaces.eth1.ipv6.addresses = [ { address = "fd00::11"; prefixLength = 64; } ];
    environment.systemPackages = [ pkgs.curl pkgs.jq pkgs.python3 pkgs.getent pkgs.acl ];
    # Open in the machine's own firewall, so that what refuses a connection to an agent's program is the fence.
    networking.firewall.allowedTCPPorts = [ 8082 ];
    # Something else listening on this machine, as any service of a person's might.
    systemd.services.local = { wantedBy = [ "multi-user.target" ]; serviceConfig.ExecStart = "${pkgs.python3}/bin/python3 -m http.server 8081 --bind :: --directory /var/empty"; };
  };

  testScript = ''
    import json, shlex
    start_all()
    internet.wait_for_unit("sites.service")
    internet.wait_for_open_port(443)
    internet.wait_for_open_port(8080)
    machine.wait_for_unit("multi-user.target")
    machine.wait_for_unit("torq-model-proxy.service")
    machine.wait_for_unit("torq-agent-network.service")
    machine.wait_for_open_port(7801)
    machine.wait_for_open_port(8081)
    machine.wait_until_succeeds("ping -c1 -W2 192.168.1.1 && ping -c1 -W2 203.0.113.10 && ping -c1 -W2 fd00::10")
    record = "/var/lib/torq-model-proxy"

    def person(who, cmd, ok=True):
        code, out = machine.execute(f"su - {who} -c {shlex.quote(cmd)} 2>&1")
        assert (code == 0) == ok, f"{who}: {cmd}: exit {code}, expected {'success' if ok else 'failure'}:\n{out}"
        return out

    def agent(who, inner, ok=True):
        return person(who, f"torq agent run -- sh -c {shlex.quote(inner)}", ok)

    def lines(who):
        text = machine.succeed(f"cat {record}/{who}/*.jsonl 2>/dev/null || true")
        return [json.loads(l) for l in text.splitlines()]

    def asked():
        return internet.succeed("cat /tmp/names-asked 2>/dev/null || true")

    direct = "curl -sS --noproxy '*' --max-time 5 -o /dev/null"

    with subtest("a person's own account reaches the network as before, and is told nothing of the proxy"):
        for url in ["http://192.168.1.1:8080/", "http://[fd00::10]:8080/", "http://127.0.0.1:8081/", "http://[::1]:8081/", "https://listed.example/", "https://unlisted.example/"]:
            # (Quoted: a person's shell is zsh, which reads the brackets of an IPv6 address as a pattern.)
            person("alice", f"{direct} '{url}'")
            person("carol", f"{direct} '{url}'")
        env = person("alice", "env")
        assert "ANTHROPIC_BASE_URL" not in env and "PROXY" not in env.upper(), env
        # No settings for Claude Code that would hold for a person's own: only a note.
        person("alice", "test ! -e /etc/claude-code/managed-settings.json && test -e /etc/claude-code/README")
        # A name a person asks for is asked of the network's DNS (the control for what an agent cannot do).
        person("alice", "getent hosts asked-by-a-person.example", ok=False)
        internet.wait_until_succeeds("grep -q asked-by-a-person /tmp/names-asked")
        person("alice", "bash -c 'echo marker-sent-by-a-person > /dev/udp/192.168.1.1/53'")
        internet.wait_until_succeeds("grep -q marker-sent-by-a-person /tmp/names-asked")

    with subtest("an agent's account reaches nothing directly: no address, IPv4 or IPv6, not this machine's other ports"):
        assert agent("alice", "id -un").strip() == "torq-agent-alice"
        for url in ["http://192.168.1.1:8080/", "http://[fd00::10]:8080/", "http://203.0.113.10:8080/", "http://127.0.0.1:8081/", "http://[::1]:8081/", "https://listed.example/", "https://api.anthropic.com/v1/messages"]:
            out = agent("alice", f"{direct} '{url}'", ok=False)
            assert "refused" in out or "Could not connect" in out or "Couldn't connect" in out or "Failed to connect" in out, f"{url}: refused at once, not left to time out: {out}"
            agent("bob", f"{direct} '{url}'", ok=False)
        # The account itself, outside the launcher too: the rule is the kernel's, by the account.
        machine.fail(f"su -s /bin/sh torq-agent-alice -c \"{direct} http://192.168.1.1:8080/\"")
        machine.fail(f"su -s /bin/sh torq-agent-bob -c \"{direct} 'http://[fd00::10]:8080/'\"")
        # And nothing comes in to a program of an agent's: its answers are refused on the way out.
        machine.succeed("su -s /bin/sh torq-agent-alice -c 'python3 -m http.server 8082 --bind 0.0.0.0 >/dev/null 2>&1 &' ; sleep 2")
        machine.fail("curl -s --noproxy '*' --max-time 3 -o /dev/null http://127.0.0.1:8082/")
        internet.fail("curl -s --max-time 3 -o /dev/null http://192.168.1.2:8082/")
        machine.succeed("pkill -u torq-agent-alice python3")
        # (The control: the same program as a person is reached, so it was the fence that refused.)
        machine.succeed("su - carol -c 'python3 -m http.server 8082 --bind 0.0.0.0 >/dev/null 2>&1 &' ; sleep 2")
        machine.succeed("curl -s --noproxy '*' --max-time 3 -o /dev/null http://127.0.0.1:8082/")
        internet.succeed("curl -s --max-time 3 -o /dev/null http://192.168.1.2:8082/")
        machine.succeed("pkill -u carol python3")

    with subtest("no DNS for an agent: not by a packet, not through the machine's services that look names up"):
        agent("alice", "echo marker-sent-by-an-agent > /dev/udp/192.168.1.1/53; true")
        out = agent("alice", "getent hosts asked-by-an-agent.example; echo getent=$?")
        assert "getent=0" not in out, out
        agent("alice", "resolvectl query asked-of-resolved.example", ok=False)
        out = agent("alice", "busctl call org.freedesktop.resolve1 /org/freedesktop/resolve1 org.freedesktop.resolve1.Manager ResolveHostname isit 0 asked-on-the-bus.example 0 0", ok=False)
        assert "denied" in out.lower() or "not permitted" in out.lower() or "rejected" in out.lower(), out
        # The same on the bus for the account itself, outside the launcher: the bus names its caller by the kernel.
        machine.fail("su -s /bin/sh torq-agent-bob -c 'busctl call org.freedesktop.resolve1 /org/freedesktop/resolve1 org.freedesktop.resolve1.Manager ResolveHostname isit 0 bob-on-the-bus.example 0 0'")
        # Its own name and the names in the machine's hosts file are still found (no service is asked).
        assert agent("alice", "id -un; getent hosts listed.example").split()[:2] == ["torq-agent-alice", "192.168.1.1"]
        # A person's question arrives after these would have: then none of the agent's is there.
        person("alice", "getent hosts asked-after-the-agent.example", ok=False)
        internet.wait_until_succeeds("grep -q asked-after-the-agent /tmp/names-asked")
        names = asked()
        for never in ["marker-sent-by-an-agent", "asked-by-an-agent", "asked-of-resolved", "asked-on-the-bus", "bob-on-the-bus"]:
            assert never not in names, f"{never} reached the network's DNS:\n{names}"

    ask = "curl -sS --max-time 20 \"$ANTHROPIC_BASE_URL/v1/messages?beta=true\" -H 'Authorization: Bearer CREDENTIAL-FOR-THE-TEST' -H 'anthropic-version: 2023-06-01' -H 'x-claude-code-session-id: session-1' -H 'x-claude-code-agent-id: helper-2' -H 'x-claude-code-parent-agent-id: helper-1' -H 'X-Person: bob' -H 'User: torq-agent-bob' -d '{\"model\":\"model-for-the-test\",\"messages\":[{\"role\":\"user\",\"content\":\"a question from WHO\"}]}'"

    with subtest("the model is reached through the proxy, and each request is recorded as the kernel names its agent"):
        out = agent("alice", ask.replace("WHO", "alice"))
        assert "hello from the model" in out and "message_delta" in out, out
        got = [json.loads(l) for l in internet.succeed("cat /tmp/model-requests").splitlines()]
        assert len(got) == 1 and got[0]["path"] == "/v1/messages?beta=true" and got[0]["host"] == "api.anthropic.com" and got[0]["version"] == "2023-06-01", got
        assert got[0]["credential_arrived"], "the credential passes through unchanged"
        assert json.loads(got[0]["body"])["messages"][0]["content"] == "a question from alice"
        mine = lines("alice")
        assert len(mine) == 1, mine
        r = mine[0]
        assert (r["v"], r["kind"], r["agent"], r["person"]) == (1, "request", "torq-agent-alice", "alice"), r
        assert (r["session"], r["agent_id"], r["parent_agent_id"]) == ("session-1", "helper-2", "helper-1"), r
        assert (r["method"], r["path"], r["model"], r["status"], r["outcome"]) == ("POST", "/v1/messages", "model-for-the-test", 200, "complete"), r
        assert r["tokens"] == {"input": 7, "cache_read": 0, "cache_write": 0, "output": 5} and r["stop"] == "end_turn", r
        assert r["new"] == [{"kind": "text", "text": "a question from alice"}] and r["answer"] == [{"kind": "text", "text": "hello from the model"}], r
        assert lines("bob") == [], "nothing is bob's because a header said bob"
        # Bob's agent, the same door, the same words: bob's.
        agent("bob", ask.replace("WHO", "bob"))
        theirs = lines("bob")
        assert len(theirs) == 1 and (theirs[0]["agent"], theirs[0]["person"], theirs[0]["new"][0]["text"]) == ("torq-agent-bob", "bob", "a question from bob"), theirs
        assert len(lines("alice")) == 1
        # No credential in the record, in any person's.
        everything = machine.succeed(f"cat {record}/*/*.jsonl")
        assert "CREDENTIAL" not in everything and "authorization" not in everything.lower() and "Bearer" not in everything, everything
        machine.fail("journalctl -u torq-model-proxy --no-pager | grep -i -e CREDENTIAL -e bearer")

    with subtest("the proxy answers the agents' accounts and no one else"):
        for who in ["alice", "carol"]:
            out = person(who, "curl -s --max-time 5 http://127.0.0.1:7801/v1/messages -d '{}'")
            assert "the agents' accounts" in out, out
        assert "the agents' accounts" in machine.succeed("curl -s --max-time 5 http://127.0.0.1:7801/v1/messages -d '{}'"), "root too"
        # From another loopback address (127.0.0.2, anyone's to bind with any port): the proxy and the secrets
        # daemon both refuse a person there, and an agent's packets from there do not leave its account at all.
        assert "the agents' accounts" in person("carol", "curl -s --max-time 5 --interface 127.0.0.2 http://127.0.0.1:7801/v1/messages -d '{}'")
        assert "127.0.0.1 only" in person("carol", "curl -s --max-time 5 --interface 127.0.0.2 http://127.0.0.1:7800/system/x/example.org/")
        assert "127.0.0.1 only" not in person("carol", "curl -s --max-time 5 http://127.0.0.1:7800/system/x/example.org/"), "(the control: from 127.0.0.1 it is not that refusal)"
        for port in ["7801", "7800"]:
            out = agent("alice", f"curl -sS --noproxy '*' --max-time 5 --interface 127.0.0.2 http://127.0.0.1:{port}/", ok=False)
            assert "TorqOS" not in out and "127.0.0.1 only" not in out, out
        internet.fail("curl -s --max-time 3 -o /dev/null http://192.168.1.2:7801/")
        assert len(lines("alice")) == 1 and len([json.loads(l) for l in internet.succeed("cat /tmp/model-requests").splitlines()]) == 2, "nothing passed on, nothing recorded"

    # What a program is told when a site is refused: asked of the proxy as a program would, the words read.
    def refused(host):
        return agent("alice", f"exec 3<>/dev/tcp/127.0.0.1/7801; printf 'CONNECT {host}:443 HTTP/1.1\\r\\nHost: {host}:443\\r\\nConnection: close\\r\\n\\r\\n' >&3; cat <&3")

    def connects(who):
        return [(l["host"], l["allowed"], l["why"]) for l in lines(who) if l["kind"] == "connect"]

    with subtest("a listed site works, an unlisted one is refused in words that name the setting, each recorded"):
        assert agent("alice", "curl -sS --max-time 20 https://listed.example/").strip() == "hello from listed.example"
        out = agent("alice", "curl -sS --max-time 20 https://unlisted.example/", ok=False)
        assert "403" in out, out
        said = refused("unlisted.example")
        assert said.startswith("HTTP/1.1 403") and "set torq agent network hosts unlisted.example" in said and "set torq agent network web true" in said, said
        said = refused("api.anthropic.com")
        assert said.startswith("HTTP/1.1 403") and "only through TorqOS's model proxy" in said and "http://127.0.0.1:7801" in said, said
        assert connects("alice") == [("listed.example", True, "listed"), ("unlisted.example", False, "not-listed"), ("unlisted.example", False, "not-listed"), ("api.anthropic.com", False, "the-model")], connects("alice")
        assert [l for l in lines("alice") if l["kind"] == "connect"][0]["reached"] is True
        # Plain HTTP is no way round: the account has no proxy for it, and the proxy carries none.
        agent("alice", "curl -sS --max-time 5 -o /dev/null http://192.168.1.1:8080/", ok=False)
        assert "HTTPS only" in agent("alice", "curl -sS --max-time 5 -x http://127.0.0.1:7801 http://192.168.1.1:8080/")

    with subtest("the record: read by its person, by no one else; written by no one but the proxy"):
        # (Each file by its own name: a pattern that matches nothing fails in the shell, which would prove nothing.)
        day = machine.succeed(f"ls {record}/alice").split()[0]
        bobs = machine.succeed(f"ls {record}/bob").split()[0]
        assert "a question from alice" in person("alice", f"cat {record}/alice/{day}")
        assert "a question from bob" in person("bob", f"cat {record}/bob/{bobs}")
        assert person("alice", f"ls {record}/alice").split() == [day]
        for other in ["bob", "carol"]:
            person(other, f"ls {record}/alice", ok=False)
            assert "denied" in person(other, f"cat {record}/alice/{day}", ok=False)
        person("carol", f"ls {record}", ok=False)
        # No agent's account: not the person's own agent, not another's; in the launcher and as the bare account.
        for who in ["alice", "bob"]:
            agent(who, f"ls {record}/alice", ok=False)
            assert "denied" in agent(who, f"cat {record}/alice/{day}", ok=False)
            agent(who, f"touch {record}/alice/planted {record}/planted", ok=False)
            machine.fail(f"su -s /bin/sh torq-agent-{who} -c 'cat {record}/alice/{day}'")
            machine.fail(f"su -s /bin/sh torq-agent-{who} -c 'cat {record}/bob/{bobs}'")
            machine.fail(f"su -s /bin/sh torq-agent-{who} -c 'ls {record}/{who}'")
        # The person reads; they do not write, remove or replace.
        person("alice", f"echo x >> {record}/alice/{day}", ok=False)
        person("alice", f"rm -f {record}/alice/{day}", ok=False)
        machine.succeed(f"test -s {record}/alice/{day}")
        person("alice", f"touch {record}/alice/new", ok=False)
        # (Where a folder has an access rule, its middle digit shows the rule's limit, not a group's access: the
        # folder's group, which no one is in, has none. `getfacl` says it whole.)
        assert "user:alice:r-x" in machine.succeed(f"getfacl -p {record}/alice") and "group::---" in machine.succeed(f"getfacl -p {record}/alice")
        # A file inherits the folder's rule, limited to reading by the file's own mode (its `mask`).
        rule = machine.succeed(f"getfacl -p {record}/alice/{day}")
        assert "user:alice:r-x" in rule and "mask::r--" in rule and "group::---" in rule and "other::---" in rule, rule
        assert "bob" not in machine.succeed(f"getfacl -p {record}/alice {record}/alice/{day}")
        assert machine.succeed(f"stat -c '%U %G %a' {record} {record}/alice {record}/alice/{day}").split("\n")[:3] == ["torq-model-proxy torq-model-proxy 711", "torq-model-proxy torq-model-proxy 750", "torq-model-proxy torq-model-proxy 640"], machine.succeed(f"stat -c '%U %G %a' {record} {record}/alice {record}/alice/{day}")
        assert machine.succeed("id -Gn torq-model-proxy").split() == ["torq-model-proxy"] and machine.succeed("getent group torq-model-proxy").strip().endswith(":")

    with subtest("Claude Code is told of the proxy by settings only an agent sees, however the engine is started"):
        want = {"ANTHROPIC_BASE_URL": "http://127.0.0.1:7801", "HTTPS_PROXY": "http://127.0.0.1:7801", "https_proxy": "http://127.0.0.1:7801", "NO_PROXY": "127.0.0.1,localhost", "no_proxy": "127.0.0.1,localhost"}
        # `torq agent run`: any program.
        out = agent("alice", "echo $ANTHROPIC_BASE_URL $HTTPS_PROXY $https_proxy; jq -c .env /etc/claude-code/managed-settings.json")
        assert out.splitlines()[0].split() == ["http://127.0.0.1:7801"] * 3 and json.loads(out.splitlines()[1]) == want, out
        # The account cannot change them: the file is not the account's, and its folder is read-only there.
        agent("alice", "echo '{}' > /etc/claude-code/managed-settings.json", ok=False)
        agent("alice", "rm /etc/claude-code/managed-settings.json", ok=False)
        # `claude`, typed plainly.
        person("alice", "mkdir -p ~/work && cd ~/work && claude")
        # The agent's service: asked, it starts the engine (the stand-in ends at once; what it wrote is the point).
        machine.execute("su - alice -c 'timeout 60 torq ask hello' >/dev/null 2>&1")
        machine.wait_until_succeeds("test -s /var/lib/torq-agent-alice/facts-service", timeout=60)
        for how in ["plain", "service"]:
            facts = dict(l.split("=", 1) for l in machine.succeed(f"cat /var/lib/torq-agent-alice/facts-{how}").splitlines())
            assert facts["base"] == "http://127.0.0.1:7801" and facts["proxy"] == "http://127.0.0.1:7801", (how, facts)
            assert json.loads(facts["managed"]) == want, (how, facts)
            assert facts["direct"] == "closed" and facts["model"] == "1" and facts["name"] == "none", (how, facts)
            assert any(l["kind"] == "request" and l.get("session") == f"from-{how}" and l["agent"] == "torq-agent-alice" for l in lines("alice")), f"the engine's own request, started {how}, is in the record"
        names = asked()
        assert "engine-plain" not in names and "engine-service" not in names, names
        unit = machine.succeed("systemctl show -p Requires -p After -p BindReadOnlyPaths -p InaccessiblePaths torq-agent@alice.service")
        assert "torq-agent-network.service" in unit and "/etc/torq/claude-code:/etc/claude-code" in unit and "/run/nscd" in unit, unit

    with subtest("the web setting opens every site, and not this machine, its own network or the model past the record"):
        machine.succeed("/run/current-system/specialisation/web/bin/switch-to-configuration test")
        machine.wait_for_unit("torq-model-proxy.service")
        machine.wait_for_open_port(7801)
        machine.wait_until_succeeds("journalctl -u torq-model-proxy --no-pager | grep -q 'the web is open'")
        assert agent("alice", "curl -sS --max-time 20 https://web.example/").strip() == "hello from web.example"
        assert agent("alice", "curl -sS --max-time 20 https://listed.example/").strip() == "hello from listed.example"
        said = refused("unlisted.example")
        assert said.startswith("HTTP/1.1 403") and "the network it sits on" in said, said
        assert refused("local.example").startswith("HTTP/1.1 403")
        assert refused("127.0.0.1").startswith("HTTP/1.1 403")
        assert refused("api.anthropic.com").startswith("HTTP/1.1 403")
        tail = connects("alice")[-6:]
        assert tail == [("web.example", True, "web"), ("listed.example", True, "listed"), ("unlisted.example", False, "address"), ("local.example", False, "address"), ("127.0.0.1", False, "address"), ("api.anthropic.com", False, "the-model")], tail
        # The fence itself is as closed as before: the web is reached through the proxy, which records each site.
        agent("alice", f"{direct} http://203.0.113.10:8080/", ok=False)
        agent("alice", f"{direct} https://web.example/", ok=False)

    with subtest("the proxy's unit has little, and its account less"):
        exposure = machine.succeed("systemd-analyze security torq-model-proxy.service | tail -1")
        print(exposure)
        assert float(exposure.split("Overall exposure level for torq-model-proxy.service:")[1].split()[0]) < 3.0, exposure
        assert machine.succeed("getent passwd torq-model-proxy").strip().split(":")[5:] == ["/var/empty", "${pkgs.shadow}/bin/nologin"]

    with subtest("only systemd holds the proxy's port, for the proxy: no one else can take it, even while the proxy is down"):
        squat = "python3 ${squat}"
        machine.succeed("systemctl stop torq-model-proxy.service")
        machine.fail("systemctl is-active torq-model-proxy.service")
        machine.succeed("systemctl is-active torq-model-proxy.socket torq-agent-network.service")
        # The proxy down, its port is still held: a person cannot take it, on 127.0.0.1 or on every address.
        for where in ["127.0.0.1", "0.0.0.0"]:
            out = person("carol", f"{squat} {where}", ok=False)
            assert "refused" in out and "in use" in out, out
        # And an agent's question starts the proxy on the socket that waited for it: answered, and recorded.
        before = len(lines("alice"))
        out = agent("alice", "curl -sS --max-time 20 \"$ANTHROPIC_BASE_URL/v1/messages\" -d '{\"model\":\"m\",\"messages\":[{\"role\":\"user\",\"content\":\"while it was down\"}]}'")
        assert "hello from the model" in out and "squatted" not in out, out
        assert len(lines("alice")) == before + 1 and lines("alice")[-1]["new"][0]["text"] == "while it was down"
        # Killed, it comes back on the same socket: the port is never free between.
        machine.succeed("systemctl kill -s KILL torq-model-proxy.service")
        assert "refused" in person("carol", f"{squat} 127.0.0.1", ok=False)
        machine.wait_until_succeeds("systemctl is-active torq-model-proxy.service")
        # Without the socket nothing of the agents is up: the fence, the agent's service, `torq agent run`.
        machine.succeed("systemctl start torq-agent@alice.service")
        machine.succeed("systemctl stop torq-model-proxy.socket")
        for unit in ["torq-model-proxy.service", "torq-agent-network.service", "torq-agent@alice.service"]:
            assert machine.succeed(f"systemctl is-active {unit} || true").strip() != "active", unit
        assert "the fence that closes your agent's network is not up" in agent("alice", "true", ok=False)
        # The port free (the control: a person can take it now), the fence cannot come up, and no agent runs.
        machine.succeed(f"su - carol -c 'nohup {squat} 127.0.0.1 hold >/tmp/squat.out 2>&1 &'")
        machine.wait_until_succeeds("grep -q bound /tmp/squat.out")
        machine.fail("systemctl start torq-agent-network.service")
        machine.fail("systemctl start torq-agent@alice.service")
        agent("alice", "true", ok=False)
        machine.succeed("pkill -u carol python3")
        machine.wait_until_fails("ss -ltn | grep -q 127.0.0.1:7801")
        machine.succeed("systemctl reset-failed; systemctl start torq-agent-network.service")
        machine.succeed("systemctl is-active torq-model-proxy.socket")
        assert agent("alice", "id -un").strip() == "torq-agent-alice"

    with subtest("nothing runs as an agent while the fence is not up"):
        machine.succeed("systemctl stop torq-agent-network.service")
        machine.fail("${pkgs.nftables}/bin/nft list table inet torq_agents")
        # What the fence was holding back (the control): the bare account now reaches the network.
        machine.succeed(f"su -s /bin/sh torq-agent-alice -c \"{direct} http://192.168.1.1:8080/\"")
        out = agent("alice", "true", ok=False)
        assert "the fence that closes your agent's network is not up" in out, out
        assert machine.succeed("systemctl is-active torq-agent@alice.service || true").strip() != "active", "the agent's service stopped with the fence"
        # The agent's service brings the fence up with it when it is started...
        machine.succeed("systemctl start torq-agent@alice.service")
        machine.succeed("systemctl is-active torq-agent-network.service && ${pkgs.nftables}/bin/nft list table inet torq_agents")
        machine.fail(f"su -s /bin/sh torq-agent-alice -c \"{direct} http://192.168.1.1:8080/\"")
        # ...and where the fence cannot be put up (here: its rules made to fail to load), the service does not start.
        machine.succeed("systemctl stop torq-agent@alice.service torq-agent-network.service")
        machine.succeed("mkdir -p /run/systemd/system/torq-agent-network.service.d && printf '[Service]\\nExecStart=\\nExecStart=${pkgs.coreutils}/bin/false\\n' > /run/systemd/system/torq-agent-network.service.d/broken.conf && systemctl daemon-reload")
        machine.fail("systemctl start torq-agent@alice.service")
        assert machine.succeed("systemctl is-active torq-agent@alice.service || true").strip() != "active"
        agent("alice", "true", ok=False)
        machine.succeed("rm -r /run/systemd/system/torq-agent-network.service.d && systemctl daemon-reload && systemctl reset-failed && systemctl start torq-agent-network.service")
        machine.fail(f"su -s /bin/sh torq-agent-alice -c \"{direct} http://192.168.1.1:8080/\"")
        assert agent("alice", "id -un").strip() == "torq-agent-alice"
  '';
}
