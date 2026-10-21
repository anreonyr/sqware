import unittest
from verify import check

PROOF = """test embedded::tests::critical_domain_exit_stops ... FAILED
timed out after 8s
[critical] armed team=3 task=9
[critical] guarded exit reason=0xfffffffe; no cleanup follows
[critical] peer harts quiesced=3
"""


class CriticalEvidence(unittest.TestCase):
    def test_complete_proof(self):
        check(PROOF, 4)

    def test_missing_or_duplicate_stage(self):
        for line in PROOF.splitlines():
            if line.startswith("[critical]"):
                with self.subTest(line=line), self.assertRaises(AssertionError):
                    check(PROOF.replace(line + "\n", ""), 4)
                with self.subTest(duplicate=line), self.assertRaises(AssertionError):
                    check(PROOF + line + "\n", 4)

    def test_missing_peer_acknowledgement(self):
        with self.assertRaises(AssertionError):
            check(PROOF.replace("quiesced=3", "quiesced=2"), 4)

    def test_reference_cleanup_is_not_a_safe_halt(self):
        with self.assertRaises(AssertionError):
            check(PROOF + "[critical] reference cleanup task=9\n", 4)

    def test_unrelated_cleanup(self):
        check(PROOF + "[critical] reference cleanup task=90\n", 4)

    def test_return_panic_or_arbitrary_timeout_is_not_a_proof(self):
        for text in ["timed out after 8s\n", PROOF + "test panic: failure\n",
                     PROOF + "critical guard returned after slay\n",
                     PROOF.replace("timed out after 8s", "test result: ok")]:
            with self.subTest(text=text), self.assertRaises(AssertionError):
                check(text, 4)
