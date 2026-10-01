"""Small, platform-independent geometry and frame diagnostics for the probe."""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import math


DEFAULT_BOARD_RECT = (0.475, 0.270, 0.485, 0.480)


def validate_board_rect(values: tuple[float, ...]) -> tuple[float, float, float, float]:
    if len(values) != 4 or not all(math.isfinite(value) for value in values):
        raise ValueError("board rect needs four finite values: x,y,width,height")
    x, y, width, height = values
    if x < 0 or y < 0 or width <= 0 or height <= 0:
        raise ValueError("board rect origin must be nonnegative and size must be positive")
    if x + width > 1.0 + 1e-9 or y + height > 1.0 + 1e-9:
        raise ValueError("board rect must stay inside the normalized client area")
    return x, y, width, height


def validate_content_aspect(values: tuple[float, ...]) -> tuple[float, float]:
    if len(values) != 2 or not all(math.isfinite(value) and value > 0 for value in values):
        raise ValueError("content aspect needs two finite positive values: width:height")
    width, height = values
    if not math.isfinite(width / height) or width / height <= 0:
        raise ValueError("content aspect ratio must be finite and positive")
    return width, height


def content_pixels(
    client_width: int, client_height: int, content_aspect: tuple[float, float] | None = None,
) -> tuple[int, int, int, int]:
    """Return content x,y,width,height inside the client, centered and bottom aligned.

    A supplied aspect is a manual MuMu calibration, not detected game geometry.
    Without it, the entire client remains the normalization source.
    """
    if client_width <= 0 or client_height <= 0:
        raise ValueError("client size must be positive")
    if content_aspect is None:
        return 0, 0, client_width, client_height
    aspect_width, aspect_height = validate_content_aspect(content_aspect)
    aspect = aspect_width / aspect_height
    if client_width / client_height >= aspect:
        width = min(client_width, max(1, round(client_height * aspect)))
        height = client_height
    else:
        width = client_width
        height = min(client_height, max(1, round(client_width / aspect)))
    return (client_width - width) // 2, client_height - height, width, height


def board_pixels(
    client_width: int,
    client_height: int,
    normalized: tuple[float, float, float, float] = DEFAULT_BOARD_RECT,
    content_aspect: tuple[float, float] | None = None,
) -> tuple[int, int, int, int]:
    """Return left, top, right, bottom in physical client pixels."""
    validate_board_rect(normalized)
    source_x, source_y, source_width, source_height = content_pixels(client_width, client_height, content_aspect)
    x, y, width, height = normalized
    left = source_x + round(source_width * x)
    top = source_y + round(source_height * y)
    right = source_x + min(source_width, round(source_width * (x + width)))
    bottom = source_y + min(source_height, round(source_height * (y + height)))
    if right <= left or bottom <= top:
        raise ValueError("board rect is smaller than one client pixel")
    return left, top, right, bottom


def grid_lines(
    rect: tuple[int, int, int, int], columns: int = 9, rows: int = 5
) -> tuple[list[int], list[int]]:
    left, top, right, bottom = rect
    if right <= left or bottom <= top or columns <= 0 or rows <= 0:
        raise ValueError("grid requires a positive rectangle and positive dimensions")
    return (
        [round(left + (right - left) * index / columns) for index in range(columns + 1)],
        [round(top + (bottom - top) * index / rows) for index in range(rows + 1)],
    )


class SampleGate:
    """Monotonic rate gate: a slow callback never produces a catch-up burst."""

    def __init__(self, fps: float) -> None:
        if not math.isfinite(fps) or not 0 < fps <= 5:
            raise ValueError("save fps must be greater than zero and at most 5")
        self.interval = 1.0 / fps
        self.last: float | None = None

    def accept(self, now: float) -> bool:
        if self.last is not None and now - self.last < self.interval:
            return False
        self.last = now
        return True


def image_slot_names(sample_index: int, max_images: int) -> list[str]:
    """Keep first and latest plus a bounded ring; never discard the first frame."""
    if sample_index < 1 or not 2 <= max_images <= 20:
        raise ValueError("sample index must be positive and max_images must be 2..20")
    names = ["latest.png"]
    if sample_index == 1:
        names.append("first.png")
    elif max_images > 2:
        names.append(f"recent_{(sample_index - 2) % (max_images - 2):02d}.png")
    return names


def rgb_sample_metrics(bgr_bytes: bytes, threshold: int = 8) -> dict:
    """Near-black heuristic on sampled BGR pixels, independent of alpha."""
    if not bgr_bytes or len(bgr_bytes) % 3:
        raise ValueError("sample must contain complete BGR pixels")
    pixels = len(bgr_bytes) // 3
    near_black = sum(
        max(bgr_bytes[offset : offset + 3]) <= threshold
        for offset in range(0, len(bgr_bytes), 3)
    )
    ratio = near_black / pixels
    return {
        "sampled_pixels": pixels,
        "mean_rgb_level": round(sum(bgr_bytes) / len(bgr_bytes), 3),
        "maximum_rgb_level": max(bgr_bytes),
        "near_black_pixel_ratio": round(ratio, 6),
        "near_black_threshold": threshold,
        "black_frame_candidate": ratio >= 0.995,
        "black_frame_method": "sampled_rgb_max_le_8_for_at_least_99.5_percent",
    }


@dataclass
class FrameDiagnostics:
    sampled_frames: int = 0
    changed_frames: int = 0
    black_frame_candidates: int = 0
    first_hash: str | None = None
    last_hash: str | None = None
    last_source_timestamp: int | None = None

    def record(self, pixel_bytes: bytes, source_timestamp: int, metrics: dict) -> dict:
        digest = hashlib.sha256(pixel_bytes).hexdigest()
        changed = self.last_hash is not None and digest != self.last_hash
        delta = None if self.last_source_timestamp is None else source_timestamp - self.last_source_timestamp
        self.sampled_frames += 1
        self.changed_frames += int(changed)
        self.black_frame_candidates += int(metrics["black_frame_candidate"])
        if self.first_hash is None:
            self.first_hash = digest
        self.last_hash = digest
        self.last_source_timestamp = source_timestamp
        return {
            "sample_index": self.sampled_frames,
            "pixel_sha256": digest,
            "changed_from_previous_sample": changed,
            "source_timestamp_100ns": source_timestamp,
            "source_timestamp_delta_100ns": delta,
            "source_timestamp_monotonic": delta is None or delta >= 0,
            **metrics,
        }

    def summary(self) -> dict:
        return {
            "successful_sampled_frames": self.sampled_frames,
            "changed_frames": self.changed_frames,
            "changed_frame_comparisons": max(0, self.sampled_frames - 1),
            "black_frame_candidates": self.black_frame_candidates,
            "first_pixel_sha256": self.first_hash,
            "last_pixel_sha256": self.last_hash,
            "black_frame_is_heuristic": True,
        }
