"""Accept a deliberately halted VM, without accepting an arbitrary timeout."""
import pathlib
import re
import sys


def one(text, pattern):
    matches = list(re.finditer(pattern, text, re.MULTILINE))
    assert len(matches) == 1, f"expected one proof: {pattern}"
    return matches[0]


def check(text, harts):
    assert harts > 1, "this proof requires peer harts"
    assert "test embedded::tests::critical_domain_exit_stops ... FAILED" in text
    assert re.search(r"timed out after \d+s", text), "VM did not remain halted"
    armed = one(text, r"^\[critical\] armed team=(\d+) task=(\d+)$")
    stopped = one(text, r"^\[critical\] guarded exit reason=0x[0-9a-f]+; no cleanup follows$")
    quiet = one(text, rf"^\[critical\] peer harts quiesced={harts - 1}$")
    assert armed.start() < stopped.start() < quiet.start(), "stop proof out of order"
    assert not re.search(rf"^\[critical\] reference cleanup task={armed[2]}$", text, re.MULTILINE), "guarded references were cleaned"
    assert "critical guard returned after slay" not in text
    assert "test panic:" not in text
    return f"critical: target {armed[2]} stopped before reference cleanup; {harts - 1} peer harts acknowledged quiescence"


if __name__ == "__main__":
    print(check(pathlib.Path(sys.argv[1]).read_text(), int(sys.argv[2])))
