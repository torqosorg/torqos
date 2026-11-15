# Claude Code, standing in as the agents' engine in the VM tests (ADR-055; the real one is unfree, large, and
# needs a sign-in): a program named `claude` that speaks what the agent's service drives the real one with —
# `claude -p --input-format stream-json --output-format stream-json --verbose …`, a question a line on its
# input, one JSON object a line on its output (agent/src/service/engine.rs has the lines, as Claude Code 2.1.316
# writes them). It is put where the real one would be (`torq.agent.claudeCode`), in each agent's own profile.
#
# Like the real one: it must be signed in (here `claude --stand-in-login`, which the test runs as the person
# would run `claude` and type /login: under `torq agent run`), and says so in the real one's words when it is
# not; it keeps its conversations in the account's home, named by a session, and continues one with `--resume`
# (an unknown one fails as the real one's does); it reads `~/.claude.json` and starts the MCP server TorqOS
# linked there, which is how it looks at the machine and proposes a change.
#
# What it answers is scripted by the question:
#   hello          "Hello alice."
#   remember?      whether `hello` was asked earlier in this conversation
#   attached?      whether the question came with something the person attached
#   look           what this account can see and write from where it runs (said as a command run)
#   alarms         TorqOS's `show_alarms`, through torq-mcp
#   install htop   TorqOS's `propose_change`, through torq-mcp
#   slow           starts programs that go where a tool's may (a session of their own, out of sight), then waits
#   die            ends in the middle of its answer
#   noise          writes what is not its protocol's (junk, an unknown kind, a line of 9 MB) before it answers
# Every start is noted in `~/.stand-in/starts` (its arguments, its account, where it runs, its environment's names).
{ pkgs }:

pkgs.writeScriptBin "claude" ''
  #!${pkgs.python3}/bin/python3
  import json, os, select, subprocess, sys, time, uuid

  home = os.environ["HOME"]
  state = os.path.join(home, ".stand-in")
  os.makedirs(os.path.join(state, "sessions"), exist_ok=True)
  args = sys.argv[1:]
  signed_in = os.path.join(state, "signed-in")

  if args == ["--stand-in-login"]:
      open(signed_in, "w").close()
      print("signed in")
      sys.exit(0)

  with open(os.path.join(state, "starts"), "a") as f:
      f.write(json.dumps({"args": args, "user": os.environ.get("USER"), "cwd": os.getcwd(), "env": sorted(os.environ)}) + "\n")

  if "-p" not in args:
      # At a terminal (`torq agent run -- claude …`, or `claude` typed plainly): it says how it was started —
      # with `--stand-in-stdin`, what its input held too — and ends, with the status after `--stand-in-exit`.
      said = {"args": args, "user": os.environ.get("USER"), "cwd": os.getcwd()}
      if "--stand-in-stdin" in args:
          said["stdin"] = sys.stdin.read()
      print(json.dumps(said))
      sys.exit(int(args[args.index("--stand-in-exit") + 1]) if "--stand-in-exit" in args else 0)

  def out(o):
      sys.stdout.write(json.dumps(o) + "\n")
      sys.stdout.flush()

  resume = args[args.index("--resume") + 1] if "--resume" in args else None
  session = resume or str(uuid.uuid4())
  kept = os.path.join(state, "sessions", session + ".json")
  asked = json.load(open(kept)) if os.path.exists(kept) else []
  lost = resume is not None and not os.path.exists(kept)

  class Mcp:
      """The MCP server TorqOS linked into ~/.claude.json, started when first used."""
      def __init__(self):
          self.p, self.n = None, 0
      def rpc(self, method, params=None, reply=True):
          if self.p is None:
              srv = json.load(open(os.path.join(home, ".claude.json")))["mcpServers"]["torqos"]
              self.p = subprocess.Popen([srv["command"]] + srv.get("args", []), stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
              self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "stand-in", "version": "0"}})
              self.rpc("notifications/initialized", reply=False)
          self.n += 1
          msg = {"jsonrpc": "2.0", "method": method}
          if params is not None:
              msg["params"] = params
          if reply:
              msg["id"] = self.n
          self.p.stdin.write(json.dumps(msg) + "\n")
          self.p.stdin.flush()
          if not reply:
              return None
          ready, _, _ = select.select([self.p.stdout], [], [], 60)
          assert ready, f"torq-mcp gave no answer to {method} within 60 s"
          return json.loads(self.p.stdout.readline())
      def call(self, name, arguments):
          return self.rpc("tools/call", {"name": name, "arguments": arguments})["result"]
  mcp = Mcp()

  def say(text):
      out({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": text}]}, "parent_tool_use_id": None, "session_id": session})

  def used(n, name, result, failed=False):
      """A tool use and its result, as the real one writes them."""
      out({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "tool_use", "id": f"toolu_{n}", "name": name, "input": {}}]}, "parent_tool_use_id": None, "session_id": session})
      out({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": f"toolu_{n}", "content": result, "is_error": failed}]}, "parent_tool_use_id": None, "session_id": session})

  def torqos(n, tool, arguments):
      r = mcp.call(tool, arguments)
      used(n, "mcp__torqos__" + tool, r["content"], r.get("isError", False))
      return r

  def attempt(f):
      try:
          return f()
      except Exception as e:
          return type(e).__name__

  def look():
      """What this account sees and may write from here."""
      def write(path):
          open(path, "w").close()
          os.unlink(path)
          return "written"
      return {
          "user": os.environ.get("USER"), "uid": os.getuid(), "home": home, "cwd": os.getcwd(), "env": sorted(os.environ),
          "/home": attempt(lambda: sorted(os.listdir("/home"))),
          "/home/alice/open": attempt(lambda: open("/home/alice/open").read().strip()),
          "/run/user": attempt(lambda: sorted(os.listdir("/run/user"))),
          "/root": attempt(lambda: sorted(os.listdir("/root"))),
          "/run/credentials": attempt(lambda: sorted(os.listdir("/run/credentials"))),
          "/tmp/outside": attempt(lambda: open("/tmp/outside").read().strip()),
          "bobs": attempt(lambda: sorted(os.listdir("/var/lib/torq-agent-bob"))),
          "write home": attempt(lambda: write(os.path.join(home, "mine"))),
          "write /etc": attempt(lambda: write("/etc/planted")),
          "write /srv/open": attempt(lambda: write("/srv/open/planted")),
          "caps": [l.split()[1] for l in open("/proc/self/status") if l.startswith(("CapEff", "CapBnd", "CapAmb"))],
          "no new privs": [l.split()[1] for l in open("/proc/self/status") if l.startswith("NoNewPrivs")],
      }

  n = 0
  for line in iter(sys.stdin.readline, ""):
      q = json.loads(line)["message"]["content"]
      n += 1
      out({"type": "system", "subtype": "init", "session_id": session, "cwd": os.getcwd(), "tools": ["Bash"], "permissionMode": "bypassPermissions"})
      if not os.path.exists(signed_in):
          out({"type": "assistant", "message": {"role": "assistant", "model": "<synthetic>", "content": [{"type": "text", "text": "Not logged in · Please run /login"}]}, "parent_tool_use_id": None, "session_id": session, "error": "authentication_failed", "is_api_error_message": True})
          out({"type": "result", "subtype": "success", "is_error": True, "result": "Not logged in · Please run /login", "session_id": session, "num_turns": 1})
          continue
      if lost:
          out({"type": "result", "subtype": "error_during_execution", "is_error": True, "errors": [f"No conversation found with session ID: {session}"], "session_id": session, "num_turns": 0})
          continue
      asked.append(q)
      json.dump(asked, open(kept, "w"))
      ask = q.split("\n")[0]
      if ask == "remember?":
          say("You said hello." if "hello" in asked[:-1] else "I remember nothing.")
      elif ask == "attached?":
          say("Attached: " + q.split("]\n", 1)[1] if "[The person attached what follows" in q else "Nothing attached.")
      elif ask == "look":
          seen = look()
          used(n, "Bash", json.dumps(seen))
          say(json.dumps(seen))
      elif ask == "alarms":
          r = torqos(n, "show_alarms", {})
          say("No alarms." if not r.get("isError") else "I could not look.")
      elif ask == "install htop":
          r = torqos(n, "propose_change", {"reason": "htop for alice", "set": [{"path": "users alice packages", "value": "htop"}]})
          say("I proposed htop." if not r.get("isError") else "I could not propose it.")
      elif ask == "slow":
          used(n, "Bash", "started")
          subprocess.Popen(["${pkgs.coreutils}/bin/sleep", "601"], start_new_session=True)
          subprocess.Popen([sys.executable, "-c", "import ctypes, time; assert ctypes.CDLL(None).prctl(4, 0, 0, 0, 0) == 0; time.sleep(602)  # undumpable-602"])
          subprocess.run(["${pkgs.coreutils}/bin/sleep", "603"])
          say("late")
      elif ask == "die":
          say("half")
          sys.exit(3)
      elif ask == "noise":
          sys.stdout.write("junk\n")
          out({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed"}})
          out({"type": "something_new", "text": "x" * (9 << 20)})
          say("After the noise.")
      else:
          say("Hello alice.")
      out({"type": "result", "subtype": "success", "is_error": False, "result": "", "session_id": session, "num_turns": 1, "total_cost_usd": 0})
''
