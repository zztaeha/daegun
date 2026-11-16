import json, sys

if len(sys.argv) < 2:
    sys.exit("pick-test-binary.py: give the cargo target name to pick, `daegun` for the library's tests or `api`")
want = sys.argv[1]

for line in sys.stdin:
    try:
        m = json.loads(line)
    except ValueError:
        continue
    if (m.get("reason") == "compiler-artifact"
            and m.get("target", {}).get("name") == want
            and m.get("executable")):
        print(m["executable"])
