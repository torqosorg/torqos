# NixOS VM test: a secret used and never read (ADR-052, step 2). The daemon, the broker and curl as a machine
# runs them; a second machine is the HTTPS host (`api.example.test`, with a certificate from a test authority
# the first machine trusts), and writes down every request it is sent, so the test can see both ends: what
# reached the host, and what came back to the program that asked. An agent's use that asks waits for its person
# (ADR-052 4): answered at a terminal, timed out, and let through unasked by a grant until the grant is ended.
# Last, the way TorqOS Settings keeps a value (step 5): usable, and in no file of the machine.
{ pkgs, self }:

let
  # The test's own passwords, as tests/socket.nix has them: dev is the owner (wheel), sam is not.
  devHash = "$6$torqsocketsalt$Z7RLey3l5vKGfukhfuZYYM/EjWNo7QM8.65r88XmZ9z3/jBCY.NAnH7RwYpiLoE2fedxUd/I7UFcKDX1KEXDg/";
  samHash = "$6$torqsocketsal2$p4ihhGlRahyPpWY43xyA52iNOzVtnXF1dQCVxf0eBeRehXODXyWOdptk3znfwCfmhj5ERP28izhmIRqrAL5J00";
  # TorqOS Settings' way of keeping a value, without the window (tests/secret-set-example.nix).
  windowSet = import ./secret-set-example.nix { inherit pkgs self; };
  certs = pkgs.runCommand "torq-secrets-test-certs" { nativeBuildInputs = [ pkgs.openssl ]; } ''
    mkdir $out
    openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj "/CN=TorqOS test authority" -keyout $out/ca.key -out $out/ca.crt
    openssl req -newkey rsa:2048 -nodes -subj "/CN=api.example.test" -keyout $out/server.key -out server.csr
    printf 'subjectAltName=DNS:api.example.test,DNS:other.example.test\n' > ext
    openssl x509 -req -in server.csr -CA $out/ca.crt -CAkey $out/ca.key -CAcreateserial -days 3650 -extfile ext -out $out/server.crt
  '';
  # The host: what it is sent, written down; what it answers, by path.
  server = pkgs.writeText "host.py" ''
    import http.server, json, ssl, gzip
    class H(http.server.BaseHTTPRequestHandler):
        def answer(self):
            n = int(self.headers.get("Content-Length") or 0)
            body = self.rfile.read(n).decode() if n else ""
            seen = {"method": self.command, "path": self.path, "headers": dict(self.headers.items()), "body": body}
            with open("/tmp/requests.log", "a") as f:
                f.write(json.dumps(seen) + "\n")
            if self.path.startswith("/redirect"):
                self.send_response(302)
                self.send_header("Location", "https://other.example.test/stolen")
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            out = json.dumps(seen).encode()
            if self.path.startswith("/gzip"):
                out = gzip.compress(out)
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            if self.path.startswith("/gzip"):
                self.send_header("Content-Encoding", "gzip")
            # An echo in a header too, as a debugging server may.
            self.send_header("X-Echo", self.headers.get("Authorization") or "")
            # As GitHub's API does: any web page may read the answer.
            if self.path.startswith("/cors"):
                self.send_header("Access-Control-Allow-Origin", "*")
            self.send_header("Content-Length", str(len(out)))
            self.end_headers()
            if self.command != "HEAD":
                self.wfile.write(out)
        do_GET = do_POST = do_PUT = do_DELETE = do_HEAD = answer
    s = http.server.ThreadingHTTPServer(("0.0.0.0", 443), H)
    c = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    c.load_cert_chain("${certs}/server.crt", "${certs}/server.key")
    s.socket = c.wrap_socket(s.socket, server_side=True)
    s.serve_forever()
  '';
in
pkgs.testers.runNixOSTest {
  name = "torq-secrets";

  nodes.far = { pkgs, ... }: {
    networking.firewall.allowedTCPPorts = [ 443 ];
    systemd.services.host = {
      wantedBy = [ "multi-user.target" ];
      serviceConfig.ExecStart = "${pkgs.python3}/bin/python3 ${server}";
    };
    # A git host over SSH: `git`, whose key is put in by the test.
    services.openssh = { enable = true; settings.PasswordAuthentication = false; };
    users.users.git = { isNormalUser = true; };
    # Light: sshd and a small server, on a laptop that runs the test.
    virtualisation.memorySize = 512;
  };

  # Another SSH host the key is let in at, which no rule names: the key must not sign for it.
  nodes.elsewhere = { ... }: {
    services.openssh = { enable = true; settings.PasswordAuthentication = false; };
    users.users.git = { isNormalUser = true; };
    # Light: sshd and a small server, on a laptop that runs the test.
    virtualisation.memorySize = 512;
  };

  nodes.machine = { pkgs, lib, nodes, ... }: {
    imports = [ self.nixosModules.default ];
    torq.user = "dev";
    # Two people's agents: uses of both wait at once (the subtest of four waiting uses).
    torq.agent.people = [ "dev" "sam" ];
    torq.desktop.enable = false;
    users.users.root.hashedPasswordFile = lib.mkForce null;
    users.users.dev.hashedPasswordFile = lib.mkForce null;
    users.users.dev.hashedPassword = devHash;
    users.users.sam = { isNormalUser = true; hashedPasswordFile = lib.mkForce null; hashedPassword = samHash; };
    torq.broker.rebuildCommand = "true";
    security.pki.certificateFiles = [ "${certs}/ca.crt" ];
    networking.hosts.${nodes.far.networking.primaryIPAddress} = [ "api.example.test" "other.example.test" "git.example.test" ];
    networking.hosts.${nodes.elsewhere.networking.primaryIPAddress} = [ "elsewhere.example.test" ];
    # tmux: a terminal for torqcli to ask a password at (a grant of secret uses).
    environment.systemPackages = [ pkgs.socat pkgs.curl pkgs.jq pkgs.tmux windowSet ];
  };

  testScript = ''
    import json, shlex
    start_all()
    far.wait_for_unit("host.service")
    far.wait_for_open_port(443)
    machine.wait_for_unit("multi-user.target")
    repo = "/var/lib/torq/config"
    machine.succeed(f"mkdir -p {repo} && echo '{{ }}' > {repo}/desktop.nix && touch {repo}/flake.nix")
    machine.succeed("torq-broker init")
    machine.wait_for_unit("torq-secrets.service")
    machine.wait_for_open_port(7800)

    def ask(user, request):
        line = json.dumps(request)
        cmd = f"printf '%s\\n' {shlex.quote(line)} | socat -t 30 - UNIX-CONNECT:/run/torq/broker.sock"
        raw = machine.succeed(f"su -s /bin/sh {user} -c {shlex.quote(cmd)}")
        code, out = None, ""
        for l in raw.splitlines():
            f = json.loads(l)
            out += f.get("out", "")
            if "exit" in f:
                code = f["exit"]
        return code, json.loads(out) if out.strip() else None

    def head():
        return machine.succeed(f"git -C {repo} rev-parse HEAD").strip()

    def requests():
        return [json.loads(l) for l in far.succeed("cat /tmp/requests.log 2>/dev/null || true").splitlines()]

    def use(user, path, *flags):
        """A request through the daemon, as `user`: (status, body, headers)."""
        args = " ".join(shlex.quote(f) for f in flags)
        cmd = f"curl -s -D /tmp/h-{user} -o /tmp/b-{user} -w '%{{http_code}}' {args} http://127.0.0.1:7800{path}"
        status = int(machine.succeed(f"su -s /bin/sh {user} -c {shlex.quote(cmd)}").strip())
        body = machine.succeed(f"cat /tmp/b-{user}")
        heads = machine.succeed(f"cat /tmp/h-{user}")
        return status, body, heads

    with subtest("dev keeps a token, and says the agent may use it for api.example.test"):
        code, v = ask("dev", {"args": ["--json", "secret-set", "--name", "gh", "--user", "dev"], "key": "dev-token-1"})
        assert code == 0, (code, v)
        # The rule says when dev is asked in a word from before 2026-11-16 (`never`: "never, unless the agent
        # read untrusted text", which every agent counts as having done). It is read as what it did: asked each time.
        rules = '{"gh": {"kind": "token", "https": {"hosts": ["api.example.test"]}, "actors": ["torq-agent"], "approve": "never"}}'
        diff = f"--- /dev/null\n+++ b/users/dev/secrets.json\n@@ -0,0 +1 @@\n+{rules}\n"
        code, p = ask("dev", {"args": ["--json", "propose", f"--base={head()}", "--reason=gh for the agent"], "diff": diff})
        assert code == 0, (code, p)
        code, v = ask("dev", {"args": ["--json", "apply", str(p["seq"])]})
        assert code == 0, (code, v)

    with subtest("its owner uses it: the host gets the value, the owner's program never sees it"):
        status, body, heads = use("dev", "/user/dev/gh/api.example.test/echo?x=1")
        assert status == 200, (status, body)
        sent = requests()[-1]
        assert sent["headers"]["Authorization"] == "Bearer dev-token-1" and sent["path"] == "/echo?x=1", sent
        assert "dev-token-1" not in body and "Bearer [secret]" in body, body
        assert "dev-token-1" not in heads and "[secret]" in heads, heads
        assert sent["headers"]["Host"] == "api.example.test", sent
        # Their own Authorization is not the one sent: the secret's takes its place. A POST's body goes as it was sent.
        status, _, _ = use("dev", "/user/dev/gh/api.example.test/echo", "-H", "Authorization: Bearer devs-own")
        assert status == 200 and requests()[-1]["headers"]["Authorization"] == "Bearer dev-token-1", requests()[-1]
        status, body, _ = use("dev", "/user/dev/gh/api.example.test/echo", "-X", "POST", "--data-binary", '{"a": 1}')
        assert status == 200 and requests()[-1]["body"] == '{"a": 1}' and requests()[-1]["method"] == "POST", requests()[-1]

    def waiting(person):
        """The uses that wait for `person`, as the broker tells them over its socket."""
        code, v = ask(person, {"args": ["--json", "secret-waiting"]})
        assert code == 0, (code, v)
        return v["waiting"]

    def begin(user, path, *flags, tag=""):
        """A request through the daemon, as `user`, left running: it waits for a person's answer."""
        args = " ".join(shlex.quote(f) for f in flags)
        cmd = f"curl -s -o /tmp/b-{user}{tag} -w '%{{http_code}}' {args} http://127.0.0.1:7800{path} > /tmp/s-{user}{tag}"
        machine.succeed(f"rm -f /tmp/s-{user}{tag}; su -s /bin/sh {user} -c {shlex.quote(cmd)} >&2 &")

    def ended(user, tag=""):
        machine.wait_until_succeeds(f"test -s /tmp/s-{user}{tag}", timeout=60)
        return int(machine.succeed(f"cat /tmp/s-{user}{tag}").strip()), machine.succeed(f"cat /tmp/b-{user}{tag}")

    with subtest("each use by dev's agent waits for dev, and only dev answers"):
        # One agent per person (ADR-051 3): `torq-agent` in dev's rules is dev's agent, torq-agent-dev. The
        # broker cannot see what an agent's programs read, so each use waits at the broker for dev (ADR-052 4,
        # amended 2026-11-16), under a number, and sends nothing until dev says yes.
        code, s = ask("torq-agent-dev", {"args": ["--json", "session-start"]})
        assert code == 0, (code, s)
        session = s["session"]
        n = len(requests())
        begin("torq-agent-dev", "/user/dev/gh/api.example.test/echo", "-H", f"X-Torq-Session: {session}", "-H", "X-Torq-Reason: listing the repositories")
        machine.wait_until_succeeds("su - dev -c 'torqcli show system secrets waiting' | grep -q 'why you are asked: TorqOS cannot see what your agent has read'", timeout=30)
        shown = machine.succeed("su - dev -c 'torqcli show system secrets waiting'")
        assert "taint" not in shown.lower() and "untrusted" not in shown.lower(), shown
        w = waiting("dev")
        assert len(w) == 1 and w[0]["facts"]["secret"] == "gh" and w[0]["facts"]["host"] == "api.example.test" and w[0]["agent_says"] == "listing the repositories", w
        assert "tainted" not in w[0]["facts"] and "why" not in w[0]["facts"], w
        assert waiting("sam") == [], "asked of dev alone"
        assert len(requests()) == n, "nothing sent while it waits"
        # The agent's own account and another person are refused (dev is this machine's only administrator).
        for other in ["torq-agent-dev", "sam"]:
            code, v = ask(other, {"args": ["--json", "secret-approve", str(w[0]["n"])]})
            assert code == 1, (other, code, v)
        code, v = ask("dev", {"args": ["--json", "secret-deny", str(w[0]["n"])]})
        assert code == 0 and v["kind"] == "secret_use_denied" and v["approved_by"] == "dev" and v["data"]["where"] == "terminal", (code, v)
        status, body = ended("torq-agent-dev")
        assert status == 403 and "denied" in body and "dev-token-1" not in body, (status, body)
        assert len(requests()) == n, "nothing sent"
        # Approved, it is sent, once, and is on the record as approved: never the value.
        begin("torq-agent-dev", "/user/dev/gh/api.example.test/echo", "-H", f"X-Torq-Session: {session}")
        machine.wait_until_succeeds("su - dev -c 'torqcli show system secrets waiting' | grep -q 'request system secret approve'", timeout=30)
        k = waiting("dev")[0]["n"]
        out = machine.succeed(f"su - dev -c 'torqcli request system secret approve {k}'")
        assert "approved" in out, out
        status, body = ended("torq-agent-dev")
        assert status == 200 and "dev-token-1" not in body and requests()[-1]["headers"]["Authorization"] == "Bearer dev-token-1", (status, body)
        ledger = machine.succeed("torq-broker log --json")
        assert "secret_use_approved" in ledger and "dev-token-1" not in ledger and "listing the repositories" not in ledger

    with subtest("nowhere else, by no one else, and not without a session"):
        n = len(requests())
        for user, path, flags in [
            ("torq-agent-dev", "/user/dev/gh/other.example.test/echo", ["-H", f"X-Torq-Session: {session}"]),
            # No session: dev is asked, as for any use of the agent's, and with no answer it is refused at its two minutes.
            ("torq-agent-dev", "/user/dev/gh/api.example.test/echo", []),
            ("sam", "/user/dev/gh/api.example.test/echo", []),
            ("dev", "/system/gh/api.example.test/echo", []),
            ("torq-agent-dev", "/user/dev/nothing/api.example.test/echo", ["-H", f"X-Torq-Session: {session}"]),
        ]:
            status, body, _ = use(user, path, *flags)
            assert status == 403 and "dev-token-1" not in body, (user, path, status, body)
        assert len(requests()) == n, "a refused use sends nothing"
        # From another loopback address (127.0.0.2: any account may bind it, with any port, so the daemon could not
        # tell its caller by the ports): refused, even for dev's own secret, and nothing sent.
        status, body, _ = use("dev", "/user/dev/gh/api.example.test/echo", "--interface", "127.0.0.2")
        assert status == 403 and "127.0.0.1 only" in body and "dev-token-1" not in body, (status, body)
        assert len(requests()) == n, "nothing sent from 127.0.0.2"
        # Bad requests are refused before anything is asked.
        status, _, _ = use("torq-agent-dev", "/user/dev/gh/10.0.0.1/echo")
        assert status == 400, status

    with subtest("a web page the owner has open sends as the owner: what a browser sends is refused"):
        n = len(requests())
        for flags in [["-H", "Origin: https://evil.example"], ["-H", "Referer: https://evil.example/"], ["-H", "Sec-Fetch-Site: cross-site"],
                      ["-H", "Host: evil.example:7800"], ["-X", "OPTIONS"]]:
            status, body, _ = use("dev", "/user/dev/gh/api.example.test/echo", *flags)
            assert status in (400, 403) and "dev-token-1" not in body, (flags, status, body)
        assert len(requests()) == n, "nothing sent"
        # What would let a page read an answer (CORS) is not passed on.
        status, _, heads = use("dev", "/user/dev/gh/api.example.test/cors")
        assert status == 200 and "access-control" not in heads.lower(), heads

    with subtest("a redirect is not followed: the value goes to the rule's host and no other"):
        n = len(requests())
        status, _, heads = use("dev", "/user/dev/gh/api.example.test/redirect")
        assert status == 302 and "other.example.test" in heads, (status, heads)
        assert len(requests()) == n + 1 and requests()[-1]["path"] == "/redirect", requests()[-1]

    with subtest("an answer that cannot be checked for the value is not passed on"):
        status, body, _ = use("dev", "/user/dev/gh/api.example.test/gzip")
        assert status == 502 and "compressed" in body, (status, body)

    with subtest("four uses wait at once, from two agents, and hold no connection to the broker: a person's own use is served beside them"):
        # The broker's socket gives one account four connections, and every use comes from the daemon's one
        # account. A use that waits holds none: the daemon is told its number and asks again about it.
        import time
        code, v = ask("sam", {"args": ["--json", "secret-set", "--name", "gh", "--user", "sam"], "key": "sam-token-1"})
        assert code == 0, (code, v)
        diff = f"--- /dev/null\n+++ b/users/sam/secrets.json\n@@ -0,0 +1 @@\n+{rules}\n"
        code, p = ask("sam", {"args": ["--json", "propose", f"--base={head()}", "--reason=gh for sam's agent"], "diff": diff})
        assert code == 0, (code, p)
        code, v = ask("sam", {"args": ["--json", "apply", str(p["seq"])]})
        assert code == 0, (code, v)
        code, sams = ask("torq-agent-sam", {"args": ["--json", "session-start"]})
        assert code == 0, (code, sams)
        for i in range(3):
            begin("torq-agent-dev", "/user/dev/gh/api.example.test/echo", "-H", f"X-Torq-Session: {session}", tag=f"-w{i}")
        begin("torq-agent-sam", "/user/sam/gh/api.example.test/echo", "-H", f"X-Torq-Session: {sams['session']}", tag="-w3")
        for _ in range(30):
            if len(waiting("dev")) == 3 and len(waiting("sam")) == 1:
                break
            time.sleep(1)
        assert len(waiting("dev")) == 3 and len(waiting("sam")) == 1, (waiting("dev"), waiting("sam"))
        # All four wait. A person's own uses go through beside them, at once, one after another.
        n = len(requests())
        for _ in range(4):
            status, body, _ = use("dev", "/user/dev/gh/api.example.test/echo")
            assert status == 200, (status, body)
        assert len(requests()) == n + 4, "each was sent"
        # A fourth of one agent's is refused at once, in these words; it does not wait.
        status, body, _ = use("torq-agent-dev", "/user/dev/gh/api.example.test/echo", "-H", f"X-Torq-Session: {session}")
        assert status == 403 and "3 uses are waiting for you already" in body, (status, body)
        for w in waiting("dev"):
            code, v = ask("dev", {"args": ["--json", "secret-deny", str(w["n"])]})
            assert code == 0, (code, v)
        code, v = ask("sam", {"args": ["--json", "secret-approve", str(waiting("sam")[0]["n"])]})
        assert code == 0 and v["kind"] == "secret_use_approved" and v["approved_by"] == "sam", (code, v)
        for i in range(3):
            status, body = ended("torq-agent-dev", tag=f"-w{i}")
            assert status == 403 and "denied" in body, (i, status, body)
        status, body = ended("torq-agent-sam", tag="-w3")
        assert status == 200 and "sam-token-1" not in body and requests()[-1]["headers"]["Authorization"] == "Bearer sam-token-1", (status, body)

    with subtest("a grant lets the agent's uses of one secret through unasked: with dev's password, for minutes, and until dev ends it"):
        # One kind of grant (ADR-052 4, amended 2026-11-16): one named secret, for minutes.
        PASSWORD = "Torq-sock-2210"
        def entries(kind):
            return [e for e in (json.loads(l) for l in machine.succeed("torq-broker --json log --brief").splitlines()) if e["kind"] == kind]
        def grants():
            code, v = ask("sam", {"args": ["--json", "secret-grants"]})
            assert code == 0, (code, v)
            return v["grants"]
        code, s = ask("torq-agent-dev", {"args": ["--json", "session-start"]})
        assert code == 0, (code, s)
        s3 = s["session"]
        gh = "/user/dev/gh/api.example.test/echo"
        def asks_and_is_denied(tag):
            """A use of gh from s3 waits for dev, who says no at a terminal: it is refused, and nothing is sent."""
            n = len(requests())
            begin("torq-agent-dev", gh, "-H", f"X-Torq-Session: {s3}", tag=tag)
            for _ in range(30):
                if waiting("dev"):
                    break
                time.sleep(1)
            w = waiting("dev")
            assert len(w) == 1 and w[0]["facts"]["secret"] == "gh", w
            out = machine.succeed(f"su - dev -c 'torqcli request system secret deny {w[0]['n']}'")
            assert "denied" in out, out
            status, body = ended("torq-agent-dev", tag=tag)
            assert status == 403 and "denied" in body and len(requests()) == n, (status, body)
        def goes_through():
            """A use of gh from s3 is sent at once, with no one asked."""
            n = len(requests())
            status, body, _ = use("torq-agent-dev", gh, "-H", f"X-Torq-Session: {s3}")
            assert status == 200 and "dev-token-1" not in body and requests()[-1]["headers"]["Authorization"] == "Bearer dev-token-1", (status, body)
            assert waiting("dev") == [] and len(requests()) == n + 1

        # With no grant, dev is asked.
        asks_and_is_denied("-g0")
        # No password, no grant: with no terminal to ask at, torqcli gives none.
        machine.fail("su - dev -c 'torqcli request system secret allow gh for 5 user dev' < /dev/null")
        assert grants() == [] and entries("trust_granted") == []
        # The kinds of grant there were before (all of one's secrets, "this session", and the word for one that
        # held whatever the agent read) are no commands of torqcli's, and no arguments of the broker's.
        for gone in ["allow all", "allow all revoke", "allow gh session user dev", "allow gh for 5 tainted-too user dev", "approve gh never-even-tainted user dev", "approve gh first-use user dev", "approve gh never user dev"]:
            out = machine.fail(f"su - dev -c 'torqcli request system secret {gone}' 2>&1 < /dev/null")
            assert "Password" not in out, (gone, out)
        for args in [["--all", "--minutes", "5"], ["--name", "gh", "--user", "dev", "--session"], ["--name", "gh", "--user", "dev", "--minutes", "5", "--tainted-too"]]:
            code, v = ask("dev", {"args": ["--json", "secret-grant"] + args, "password": PASSWORD})
            assert code not in (0, 77), (args, code, v)
        assert grants() == [] and entries("trust_granted") == []
        # At a terminal, torqcli asks dev's password there, and the grant is on the record with what it covers.
        machine.succeed("su - dev -c \"tmux new-session -d -s g -x 200 -y 50 'torqcli request system secret allow gh for 5 user dev; sleep 600'\"")
        machine.wait_until_succeeds("su - dev -c 'tmux capture-pane -p -t g' | grep -c 'Password for dev:' > /dev/null", timeout=60)
        machine.succeed(f"su - dev -c 'tmux send-keys -t g {PASSWORD} Enter'")
        machine.wait_until_succeeds("su - dev -c 'tmux capture-pane -p -t g' | grep -c 'without asking, for 5 minutes' > /dev/null", timeout=60)
        machine.succeed("su - dev -c 'tmux kill-session -t g'")
        first_grant = entries("trust_granted")[-1]
        assert first_grant["approved_by"] == "dev" and first_grant["data"]["person"] == "dev" and first_grant["data"]["covers"] == "secret gh" and first_grant["data"]["minutes"] == 5 and first_grant["data"]["owner"] == "user", first_grant
        held = grants()
        assert len(held) == 1 and held[0]["seq"] == first_grant["seq"] and held[0]["person"] == "dev" and held[0]["covers"] == {"name": "gh", "system": False}, held
        assert "tainted_too" not in held[0] and "session" not in held[0], held
        shown = machine.succeed("su - sam -c 'torqcli show system secrets allowed'")
        assert "dev's agent: their secret gh, without asking until" in shown and "UNTRUSTED" not in shown.upper() and "taint" not in shown.lower(), shown
        # Now the same use goes through at once, with no one asked, and is on the record as under that grant.
        goes_through()
        under = [e for e in entries("secret_first_use") if e["data"].get("grant") == first_grant["seq"]]
        assert len(under) == 1 and under[0]["data"]["session"] == s3 and under[0]["tainted"], under
        # One grant of a thing at a time: a second is refused while the first holds.
        grant = {"args": ["--json", "secret-grant", "--name", "gh", "--user", "dev", "--minutes", "5"], "password": PASSWORD}
        code, v = ask("dev", grant)
        assert code == 1 and "revoke it first" in v["error"], (code, v)
        # Ended, by name and with no password, as dev's own (`user dev`): with no `user` the line names the system's
        # secret, and torqcli says the line dev meant; at the socket, the system's of that name is another grant,
        # of which dev has none, and another person's is refused. A secret dev never granted has nothing to end.
        machine.fail("su - dev -c 'torqcli request system secret allow revoke gh'")
        code, v = ask("dev", {"args": ["--json", "secret-revoke", "--name", "gh"]})
        assert code == 0 and v["revoked"] == [], (code, v)
        code, v = ask("sam", {"args": ["--json", "secret-revoke", "--name", "gh", "--user", "dev"]})
        assert code == 1, (code, v)
        assert [g["seq"] for g in grants()] == [first_grant["seq"]], grants()
        out = machine.succeed("su - dev -c 'torqcli request system secret allow revoke gh user dev'")
        assert "revoked" in out, out
        ended_grants = entries("trust_revoked")
        assert ended_grants[-1]["data"]["grant"] == first_grant["seq"] and ended_grants[-1]["approved_by"] == "dev", ended_grants[-1]
        assert grants() == []
        assert "no such grant" in machine.succeed("su - dev -c 'torqcli request system secret allow revoke deploy'")
        # Ended, the next use asks again.
        asks_and_is_denied("-g1")

        # Only with dev's own password, only dev's own to give, only for 1 to 480 minutes.
        code, v = ask("dev", dict(grant, password="wrong"))
        assert code == 77, (code, v)
        for who, request in [
            ("torq-agent-dev", {"args": grant["args"]}),
            ("sam", {"args": grant["args"], "password": "Sam-sock-7742"}),
            ("dev", {"args": ["--json", "secret-grant", "--name", "gh", "--user", "dev", "--minutes", "481"], "password": PASSWORD}),
            ("dev", {"args": ["--json", "secret-grant", "--name", "gh", "--user", "dev", "--minutes", "0"], "password": PASSWORD}),
        ]:
            code, v = ask(who, request)
            assert code not in (0, 77), (who, request["args"], code, v)
        assert grants() == [], grants()
        code, v = ask("dev", grant)
        assert code == 0 and v["kind"] == "trust_granted" and v["data"]["covers"] == "secret gh", (code, v)
        goes_through()
        n = len(requests())
        # A grant widens no rule: another host is still refused, and a secret it does not name still asks (sam's
        # agent, for sam's own gh, is no part of dev's grant: it waits for sam, who says no).
        status, body, _ = use("torq-agent-dev", "/user/dev/gh/other.example.test/echo", "-H", f"X-Torq-Session: {s3}")
        assert status == 403 and len(requests()) == n, (status, body)
        begin("torq-agent-sam", "/user/sam/gh/api.example.test/echo", "-H", f"X-Torq-Session: {sams['session']}", tag="-g3")
        for _ in range(30):
            if waiting("sam"):
                break
            time.sleep(1)
        code, d = ask("sam", {"args": ["--json", "secret-deny", str(waiting("sam")[0]["n"])]})
        assert code == 0, (code, d)
        status, body = ended("torq-agent-sam", tag="-g3")
        assert status == 403 and len(requests()) == n, (status, body)
        # Everything dev allowed is ended in one line, and the next use asks again.
        out = machine.succeed("su - dev -c 'torqcli request system secret allow revoke all'")
        assert "revoked" in out, out
        assert grants() == []
        asks_and_is_denied("-g2")

    with subtest("without-asking is dev's own word for their own secret: set with their password, it lets the agent's uses through; a word from before never does"):
        def rule_of_gh(word, reason):
            """Propose dev's rules with gh's `approve` as `word`: the proposal."""
            now = machine.succeed(f"git -C {repo} show HEAD:users/dev/secrets.json").rstrip("\n")
            assert "\n" not in now, now
            new = json.loads(now)
            new["gh"]["approve"] = word
            diff = f"--- a/users/dev/secrets.json\n+++ b/users/dev/secrets.json\n@@ -1 +1 @@\n-{now}\n+{json.dumps(new)}\n"
            code, p = ask("dev", {"args": ["--json", "propose", f"--base={head()}", f"--reason={reason}"], "diff": diff})
            assert code == 0, (code, p)
            return p
        p = rule_of_gh("without-asking", "gh without asking")
        assert p["data"]["standing"] == ["users/dev/secrets.json: gh"], p
        # A change that makes a secret never ask is dev's own, with their password: not without it, not with a
        # wrong one, and not sam's (with sam's own password).
        # (77: the socket's word for "this needs a password", and for a wrong one.)
        code, v = ask("dev", {"args": ["--json", "apply", str(p["seq"])]})
        assert code == 77 and v["password"] is True, (code, v)
        code, v = ask("dev", {"args": ["--json", "apply", str(p["seq"])], "password": "wrong"})
        assert code == 77, (code, v)
        code, v = ask("sam", {"args": ["--json", "apply", str(p["seq"])], "password": "Sam-sock-7742"})
        assert code != 0, (code, v)
        assert json.loads(machine.succeed(f"git -C {repo} show HEAD:users/dev/secrets.json"))["gh"]["approve"] == "never", "not applied"
        asks_and_is_denied("-n0")
        code, v = ask("dev", {"args": ["--json", "apply", str(p["seq"])], "password": PASSWORD})
        assert code == 0, (code, v)
        assert grants() == []
        goes_through()
        shown = machine.succeed("su - dev -c 'torqcli show system secrets'")
        assert "approved without-asking [!! NEVER ASKS YOU]" in shown, shown
        # A grant widens no rule, and neither does this: another host is still refused.
        status, body, _ = use("torq-agent-dev", "/user/dev/gh/other.example.test/echo", "-H", f"X-Torq-Session: {s3}")
        assert status == 403, (status, body)
        # The system's rules cannot say it for the people whose agents they name.
        system_rules = '{"ci": {"kind": "token", "https": {"hosts": ["api.example.test"]}, "actors": ["torq-agent-dev"], "approve": "without-asking"}}'
        diff = f"--- /dev/null\n+++ b/secrets/system.json\n@@ -0,0 +1 @@\n+{system_rules}\n"
        code, v = ask("dev", {"args": ["--json", "propose", f"--base={head()}", "--reason=ci unasked"], "diff": diff, "password": PASSWORD})
        assert code not in (0, 77) and "not the system's" in v["error"], (code, v)
        # Taken back in a word from before 2026-11-16 (`first-use`: once asked the first time in a session, and
        # every time since each agent counts as having read untrusted text). No password: it lets nothing
        # through. It reads as asked each time, never as anything looser.
        back = rule_of_gh("first-use", "gh asks again")
        assert "standing" not in back["data"], back
        code, v = ask("dev", {"args": ["--json", "apply", str(back["seq"])]})
        assert code == 0, (code, v)
        asks_and_is_denied("-n1")
        asks_and_is_denied("-n2")
        shown = machine.succeed("su - dev -c 'torqcli show system secrets'")
        assert "approved each-use" in shown and "NEVER ASKS" not in shown, shown

    with subtest("with no answer in two minutes the use is refused, and recorded as timed out, with why it was asked"):
        code, _ = ask("torq-agent-dev", {"args": ["--json", "session-taint", session, "--source", "web"]})
        assert code == 0
        status, body, _ = use("torq-agent-dev", "/user/dev/gh/api.example.test/echo", "-H", f"X-Torq-Session: {session}")
        assert status == 403 and "no one answered" in body and "TorqOS cannot see what an agent has read" in body, (status, body)
        assert "secret_use_timed_out" in machine.succeed("torq-broker log --json")

    with subtest("an SSH key, from a file, for the host the person checked: its key in the rule (step 3)"):
        machine.succeed("su - dev -c " + shlex.quote("mkdir -p ~/.ssh && ssh-keygen -q -t ed25519 -N \"\" -C deploy -f ~/.ssh/deploy"))
        public = machine.succeed("cat /home/dev/.ssh/deploy.pub").strip()
        for host in [far, elsewhere]:
            host.wait_for_unit("sshd.service")
            host.succeed(f"install -d -o git -m 700 /home/git/.ssh && echo {shlex.quote(public)} > /home/git/.ssh/authorized_keys && chown git /home/git/.ssh/authorized_keys")
        # dev checks git.example.test's key once, as a person does (their own known_hosts), and only that host.
        machine.succeed("su - dev -c 'ssh-keyscan -t ed25519 git.example.test > ~/.ssh/known_hosts 2>/dev/null'")
        machine.succeed("su - dev -c 'torq request system secret set deploy file /home/dev/.ssh/deploy user dev'")
        machine.succeed("su - dev -c 'torq request system secret allow deploy ssh git.example.test user dev'")
        machine.fail("su - dev -c 'torq request system secret allow deploy ssh elsewhere.example.test user dev'")
        machine.succeed("su - dev -c 'torq request system secret allow deploy agent user dev'")
        rules = json.loads(machine.succeed(f"git -C {repo} show HEAD:users/dev/secrets.json"))
        # A new rule asks each time. And torqcli wrote the file in today's words: gh's word from before is gone.
        assert rules["deploy"]["approve"] == "each-use" and rules["gh"]["approve"] == "each-use", rules
        assert rules["deploy"]["ssh"]["hosts"] == ["git.example.test"] and rules["deploy"]["ssh"]["known"][0].startswith("git.example.test ssh-ed25519 "), rules
        assert machine.succeed("stat -c %a /var/lib/torq/secrets/store/users/dev/deploy.pub").strip() == "644"
        shown = machine.succeed("su - dev -c 'torq show system secrets'")
        assert "deploy" in shown and "SSH key, has a value; ssh to git.example.test" in shown, shown

    with subtest("dev's agent account gets its own agent socket, its key listed, and each signature waits for dev"):
        code, s = ask("torq-agent-dev", {"args": ["--json", "session-start"]})
        session2 = s["session"]
        out = machine.succeed(f"su -s /bin/sh torq-agent-dev -c \"curl -s -X POST -H 'X-Torq-Session: {session2}' http://127.0.0.1:7800/ssh-agent\"")
        sock = json.loads(out)["socket"]
        assert sock.startswith("/run/torq-secrets/ssh-"), out
        machine.succeed("cp /home/dev/.ssh/known_hosts /tmp/agent-known && chmod 644 /tmp/agent-known")
        # The agent's account reaches no SSH host itself: the fence closes its network but for the model proxy and
        # this daemon (ADR-057; `ssh` as the agent is refused at once). What is tested here is the signature's wait
        # for dev, so the connection is made for it by root, through a unix socket only this test opens.
        machine.succeed("systemd-run --unit ssh-for-the-test $(command -v socat) UNIX-LISTEN:/run/ssh-for-the-test.sock,fork,mode=0666 TCP:git.example.test:22")
        machine.wait_for_file("/run/ssh-for-the-test.sock")
        machine.fail("su -s /bin/sh torq-agent-dev -c 'ssh -o BatchMode=yes -o ConnectTimeout=5 git@git.example.test true' 2>&1")
        ssh = f"SSH_AUTH_SOCK={sock} ssh -o BatchMode=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile=/tmp/agent-known -o IdentitiesOnly=no -o ProxyCommand=\"socat - UNIX-CONNECT:/run/ssh-for-the-test.sock\""
        listed = machine.succeed(f"su -s /bin/sh torq-agent-dev -c 'SSH_AUTH_SOCK={sock} ssh-add -l'")
        assert "torq:dev/deploy" in listed, listed
        # Each signature of the agent's waits for dev, who says no here.
        machine.succeed(f"rm -f /tmp/ssh-agent-dev; su -s /bin/sh torq-agent-dev -c '{ssh} git@git.example.test echo signed-in; echo $? > /tmp/ssh-agent-dev' >&2 &")
        machine.wait_until_succeeds("su - dev -c 'torqcli show system secrets waiting' | grep -q 'to sign in over SSH to git.example.test'", timeout=60)
        w = waiting("dev")
        assert len(w) == 1 and w[0]["facts"]["secret"] == "deploy" and w[0]["facts"]["ssh"], w
        code, v = ask("dev", {"args": ["--json", "secret-deny", str(w[0]["n"])]})
        # Denied by dev for this secret, and on the record (a failed ssh alone could be anything).
        assert code == 0 and v["kind"] == "secret_use_denied" and v["data"]["secret"] == "deploy" and v["data"]["user"] == "dev", (code, v)
        machine.wait_until_succeeds("test -s /tmp/ssh-agent-dev", timeout=60)
        assert machine.succeed("cat /tmp/ssh-agent-dev").strip() != "0", "not signed in"
        journal = machine.succeed("journalctl -u torq-secrets --no-pager")
        assert "torq-agent-dev: " in journal and "denied" in journal, journal
        ledger = machine.succeed("torq-broker log")
        assert "dev's secret deploy for the first time" not in ledger and "PRIVATE KEY" not in ledger, ledger
        # The socket answers its own account alone; it takes no key, nor gives one.
        machine.fail(f"su -s /bin/sh sam -c 'SSH_AUTH_SOCK={sock} ssh-add -l'")
        machine.succeed("su -s /bin/sh torq-agent-dev -c 'ssh-keygen -q -t ed25519 -N \"\" -f /tmp/agents-own'")
        machine.fail(f"su -s /bin/sh torq-agent-dev -c 'SSH_AUTH_SOCK={sock} ssh-add /tmp/agents-own'")
        assert "PRIVATE KEY" not in machine.succeed("journalctl -u torq-secrets --no-pager")
        machine.succeed("systemctl stop ssh-for-the-test")

    with subtest("its owner signs in with it the same way"):
        out = machine.succeed("su - dev -c 'curl -s -X POST http://127.0.0.1:7800/ssh-agent'")
        sock = json.loads(out)["socket"]
        assert machine.succeed(f"su - dev -c 'SSH_AUTH_SOCK={sock} ssh -o BatchMode=yes git@git.example.test echo mine'").strip() == "mine"

    with subtest("no one but the daemon asks the broker for a use, and the value is nowhere it can be read"):
        for user in ["torq-agent-dev", "dev", "sam"]:
            code, v = ask(user, {"args": ["--json", "secret-use", "--name", "gh", "--user", "dev", "--host", "api.example.test", "--client-port", "1", "--server-port", "7800"]})
            assert code == 1, (user, code, v)
        pid = machine.succeed("systemctl show -p MainPID --value torq-secrets").strip()
        machine.fail(f"su -s /bin/sh torq-agent-dev -c 'cat /proc/{pid}/environ'")
        machine.fail("su -s /bin/sh torq-agent-dev -c 'ls /var/lib/torq/secrets/store'")
        journal = machine.succeed("journalctl -u torq-secrets --no-pager")
        assert "dev-token-1" not in journal, "never in the daemon's log"
        assert "dev used dev's secret gh" in journal

    with subtest("a value kept the way TorqOS Settings keeps it is usable, and is in no file"):
        # The window's own call (app::secret_set), as dev: over the socket, the value the request's own.
        keep = "su -s /bin/sh {} -c 'torq-secret-set-example {}'"
        said = machine.succeed("printf %s window-token-7 | " + keep.format("dev", "win dev"))
        assert "The value of dev's secret win is kept" in said and "window-token-7" not in said, said
        # Kept by the broker, encrypted: what is in the store is not the value, and decrypts to it.
        store = "/var/lib/torq/secrets/store/users/dev/win"
        assert machine.succeed(f"stat -c %a:%U {store}").strip() == "600:root"
        assert machine.succeed(f"systemd-creds decrypt --name=torq-secret.user.dev.win {store} -") == "window-token-7"
        # Usable: the host is sent it, the program that asked is not.
        machine.succeed("su - dev -c 'torq request system secret allow win https api.example.test user dev'")
        status, body, _ = use("dev", "/user/dev/win/api.example.test/echo")
        assert status == 200 and "window-token-7" not in body, (status, body)
        assert requests()[-1]["headers"]["Authorization"] == "Bearer window-token-7", requests()[-1]
        # In no file of the machine's, the ledger and the logs among them (dev's own torqcli log is in /home).
        found = machine.succeed("grep -rlaF -D skip window-token-7 /home /root /tmp /run /dev/shm /var /etc 2>/dev/null || true")
        assert found.strip() == "", found
        assert "window-token-7" not in machine.succeed("torq-broker log")
        assert "window-token-7" not in machine.succeed("journalctl --no-pager -b")
        # Who may is the broker's to say, this way as any: not another person's, and not the system's for one who is no
        # administrator; nothing typed is no value. What was kept stays.
        machine.fail("printf %s window-token-8 | " + keep.format("sam", "win dev"))
        machine.fail("printf %s window-token-8 | " + keep.format("sam", "win"))
        # dev is an administrator (the owner, in wheel), so the system's is not refused outright: it needs dev's
        # password. This machine has no desktop, so the broker asks no dialog and answers at once that it needs
        # one; there is no terminal to ask at, and the call ends there. Bounded all the same: a wait is a failure.
        code, out = machine.execute("printf %s window-token-8 | timeout 30 " + keep.format("dev", "win") + " 2>&1")
        assert code == 1 and "needs dev's password" in out, (code, out)
        machine.fail(": | " + keep.format("dev", "win dev"))
        assert machine.succeed(f"systemd-creds decrypt --name=torq-secret.user.dev.win {store} -") == "window-token-7"
        machine.fail("test -e /var/lib/torq/secrets/store/system/win")
  '';
}
