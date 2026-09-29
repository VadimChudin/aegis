import datetime as dt
import unittest

try:
    from aegis_lab.research.gold_parts import parse_meeting
except ImportError:  # research extras (pandas, numpy) not installed
    parse_meeting = None


@unittest.skipIf(parse_meeting is None, "research extras not installed")
class ParseMeetingTest(unittest.TestCase):
    def test_title_formats(self):
        self.assertEqual(parse_meeting("January 29-30 Meeting - 2013"), dt.date(2013, 1, 30))
        self.assertEqual(parse_meeting("March 13 Meeting - 2012"), dt.date(2012, 3, 13))
        self.assertEqual(parse_meeting("July 31-August 1  Meeting - 2012"), dt.date(2012, 8, 1))
        self.assertEqual(parse_meeting("April/May 30-1 Meeting - 2013"), dt.date(2013, 5, 1))
        self.assertEqual(parse_meeting("Jan/Feb 31-1 Meeting - 2017"), dt.date(2017, 2, 1))

    def test_skips_unscheduled(self):
        for t in ("October 16 (unscheduled) - 2013", "March 15 (unscheduled) Meeting - 2020",
                  "March 17-18 (cancelled) Meeting - 2020", "March 19 (notation vote) - 2020"):
            self.assertIsNone(parse_meeting(t), t)


if __name__ == "__main__":
    unittest.main()
