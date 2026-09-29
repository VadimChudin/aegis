import unittest

try:
    import numpy as np

    from aegis_lab.research.flush_report import _events
except ImportError:  # research extras (numpy, pandas, numba) not installed
    _events = None


@unittest.skipIf(_events is None, "research extras not installed")
class EventsTest(unittest.TestCase):
    def run_events(self, tail):
        x = np.array([100.0] * 10 + [99.6, 99.0] + tail)
        rmax = np.array([x[max(0, k - 10): k + 1].max() for k in range(len(x))])
        return _events(x, rmax, np.ones(len(x), np.bool_), 0.005, 10, 100)

    def test_buyback(self):
        # trigger at 99.0 (leg 1.0 from the high at index 9), back to 99.5 before 98.5
        trig, high, res, out = self.run_events([99.2, 99.5, 99.0])
        self.assertEqual((trig[0], high[0], res[0], out[0]), (11, 9, 13, 1))

    def test_continuation(self):
        trig, _, res, out = self.run_events([99.3, 98.8, 98.5, 99.9])
        self.assertEqual((trig[0], res[0], out[0]), (11, 14, 0))

    def test_unresolved(self):
        trig, _, _, out = self.run_events([99.1] * 5)
        self.assertEqual((len(trig), out[0]), (1, -1))


if __name__ == "__main__":
    unittest.main()
