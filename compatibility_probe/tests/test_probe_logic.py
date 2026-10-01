import unittest

from probe_logic import (FrameDiagnostics, SampleGate, board_pixels, content_pixels, grid_lines,
                         image_slot_names, rgb_sample_metrics, validate_board_rect)


class GeometryTests(unittest.TestCase):
    def test_sample_board_and_grid(self):
        rect = board_pixels(1920, 1080)
        self.assertEqual(rect, (912, 292, 1843, 810))
        vertical, horizontal = grid_lines(rect)
        self.assertEqual(len(vertical), 10)
        self.assertEqual(len(horizontal), 6)
        self.assertEqual((vertical[0], horizontal[0], vertical[-1], horizontal[-1]), rect)

    def test_tiny_or_invalid_rect_is_rejected(self):
        for value in ((0, 0, 1.1, 1), (0, 0, 0, 1), (float("nan"), 0, 1, 1), (0, 0, 1)):
            with self.subTest(value=value), self.assertRaises(ValueError):
                validate_board_rect(value)
        with self.assertRaises(ValueError):
            board_pixels(1, 1, (0, 0, 0.001, 0.001))

    def test_mumu_aspect_calibration_removes_window_toolbar_and_fits_fullscreen(self):
        aspect = (16, 9)
        self.assertEqual(content_pixels(1920, 1140, aspect), (0, 60, 1920, 1080))
        self.assertEqual(content_pixels(3840, 2160, aspect), (0, 0, 3840, 2160))
        entire_content = (0, 0, 1, 1)
        self.assertEqual(board_pixels(1920, 1140, entire_content, aspect), (0, 60, 1920, 1140))
        self.assertEqual(board_pixels(3840, 2160, entire_content, aspect), (0, 0, 3840, 2160))
        self.assertEqual(content_pixels(1920, 1140), (0, 0, 1920, 1140))
        self.assertEqual(content_pixels(2000, 1000, aspect), (111, 0, 1778, 1000))


class DiagnosticTests(unittest.TestCase):
    def test_black_heuristic_and_changes_have_explicit_denominator(self):
        diagnostics = FrameDiagnostics()
        black = bytes([0, 0, 0] * 4)
        colored = bytes([0, 0, 220] * 4)
        first = diagnostics.record(black, 100, rgb_sample_metrics(black))
        same = diagnostics.record(black, 200, rgb_sample_metrics(black))
        changed = diagnostics.record(colored, 300, rgb_sample_metrics(colored))
        self.assertTrue(first["black_frame_candidate"])
        self.assertFalse(first["changed_from_previous_sample"])
        self.assertFalse(same["changed_from_previous_sample"])
        self.assertTrue(changed["changed_from_previous_sample"])
        self.assertFalse(changed["black_frame_candidate"])
        self.assertEqual(changed["source_timestamp_delta_100ns"], 100)
        self.assertEqual(diagnostics.summary()["changed_frames"], 1)
        self.assertEqual(diagnostics.summary()["changed_frame_comparisons"], 2)
        self.assertEqual(diagnostics.summary()["black_frame_candidates"], 2)

    def test_rate_gate_has_no_catch_up_burst(self):
        gate = SampleGate(5)
        self.assertTrue(gate.accept(0))
        self.assertFalse(gate.accept(0.199))
        self.assertTrue(gate.accept(0.201))
        self.assertTrue(gate.accept(2))
        self.assertFalse(gate.accept(2.001))
        with self.assertRaises(ValueError):
            SampleGate(5.01)

    def test_retention_never_grows_past_configured_png_count(self):
        for maximum in (2, 3, 6, 20):
            retained = set()
            for index in range(1, 500):
                retained.update(image_slot_names(index, maximum))
            self.assertEqual(len(retained), maximum)
            self.assertIn("first.png", retained)
            self.assertIn("latest.png", retained)

    def test_corrupt_samples_and_reversed_timestamps(self):
        with self.assertRaises(ValueError):
            rgb_sample_metrics(b"\x00\x00")
        diagnostics = FrameDiagnostics()
        pixels = b"\x00\x00\x00"
        diagnostics.record(pixels, 20, rgb_sample_metrics(pixels))
        record = diagnostics.record(pixels, 10, rgb_sample_metrics(pixels))
        self.assertFalse(record["source_timestamp_monotonic"])


if __name__ == "__main__":
    unittest.main()
