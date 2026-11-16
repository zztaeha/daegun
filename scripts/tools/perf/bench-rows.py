"""benchmark.md's operations against what times them: each row one of daegun's latency tests, so a renamed test can't
leave a row behind, and no path a reader of the repository lacks."""
import re
from pathlib import Path

table = Path("benchmark.md").read_text(encoding="utf-8")
rows = re.findall(r"^\| \d+ \| `([^`]+)` \|", table, re.M)
if not rows:
    raise SystemExit("benchmark.md has no operation rows")
if "tasks/" in table:
    raise SystemExit("benchmark.md names a path under tasks/, which isn't in the repository")
ours = "\n".join(p.read_text(encoding="utf-8") for p in Path("src").rglob("*latency*.rs"))
for name in rows:
    if not re.search(rf"(?<![\w ]){re.escape(name)}(?![\w ])", ours):
        raise SystemExit(f"{name}: in benchmark.md, not in daegun's latency tests")
print(f"{len(rows)} operations, each one of daegun's latency tests")
