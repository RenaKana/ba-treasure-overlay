"""Isolated WGC worker. The supervisor can always stop its own child process."""

from __future__ import annotations

from datetime import datetime, timezone
import hashlib
import importlib.metadata
import json
import math
import os
from pathlib import Path
import signal
import threading
import time

from probe_logic import FrameDiagnostics, SampleGate, image_slot_names, rgb_sample_metrics


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds")


def atomic_bytes(path: Path, content: bytes) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_bytes(content)
    os.replace(temporary, path)


def atomic_json(path: Path, value: dict) -> None:
    atomic_bytes(path, json.dumps(value, ensure_ascii=False, indent=2).encode("utf-8"))


def capture_worker(target_hwnd: int, output_directory: str, seconds: float, fps: float,
                   max_images: int, stop_event) -> None:
    # Ctrl+C belongs to the supervisor, which requests a stop and has a bounded join.
    signal.signal(signal.SIGINT, signal.SIG_IGN)
    output = Path(output_directory)
    state_path = output / "capture_status.json"
    state = {
        "api": "Windows.Graphics.Capture", "target_type": "window_hwnd",
        "target_hwnd_hex": f"0x{target_hwnd:X}", "status": "initializing",
        "requested_save_fps": fps, "maximum_retained_png_files": max_images,
        "native_minimum_update_interval": None,
        "rate_limit_scope": "sample_diagnostics_and_png_saves_not_native_callback_delivery",
        "started_utc": utc_now(), "callbacks_received": 0, "frames": [],
        "saved_slots": {}, "errors": [], "cleanup_complete": False,
        "true_exclusive_fullscreen_tested": False,
    }
    atomic_json(state_path, state)
    control = None
    started = time.monotonic()
    lock = threading.RLock()
    closed = threading.Event()
    diagnostics = FrameDiagnostics()
    gate = SampleGate(fps)

    def save_status() -> None:
        with lock:
            state.update(diagnostics.summary())
            state["elapsed_seconds"] = round(time.monotonic() - started, 3)
            atomic_json(state_path, state)

    try:
        import cv2
        from windows_capture import WindowsCapture

        state["package_versions"] = {
            name: importlib.metadata.version(name)
            for name in ("windows-capture", "numpy", "opencv-python")
        }
        if state["package_versions"]["windows-capture"] != "2.0.1":
            raise RuntimeError("Expected windows-capture==2.0.1; install requirements.txt in the probe venv")
        capture = WindowsCapture(
            window_hwnd=target_hwnd, cursor_capture=False, draw_border=None,
            secondary_window=False, minimum_update_interval=None, dirty_region=None,
        )

        @capture.event
        def on_frame_arrived(frame, capture_control):
            with lock:
                state["callbacks_received"] += 1
                if stop_event.is_set() or time.monotonic() - started >= seconds:
                    capture_control.stop()
                    return
                now = time.monotonic()
                if not gate.accept(now):
                    return
                try:
                    buffer = frame.frame_buffer
                    if frame.width <= 0 or frame.height <= 0 or buffer.ndim != 3 or buffer.shape[2] < 3:
                        raise RuntimeError("WGC returned invalid frame dimensions or color channels")
                    # Only WGC-provided pixels are accessed; there is no game-process handle.
                    bgr = buffer[:, :, :3].copy()
                    step_y = max(1, math.ceil(frame.height / 64))
                    step_x = max(1, math.ceil(frame.width / 64))
                    metrics = rgb_sample_metrics(bgr[::step_y, ::step_x].tobytes())
                    encoded_ok, encoded = cv2.imencode(".png", bgr)
                    if not encoded_ok:
                        raise RuntimeError("OpenCV could not encode the WGC frame as PNG")
                    png_bytes = encoded.tobytes()
                    next_index = diagnostics.sampled_frames + 1
                    slots = image_slot_names(next_index, max_images)
                    # Path.write_bytes handles Chinese paths; cv2.imwrite on Windows may not.
                    for name in slots:
                        atomic_bytes(output / name, png_bytes)
                    record = diagnostics.record(bgr.tobytes(), int(frame.timespan), metrics)
                    record.update({"received_utc": utc_now(), "elapsed_seconds": round(now - started, 6),
                                   "width": int(frame.width), "height": int(frame.height),
                                   "png_sha256": hashlib.sha256(png_bytes).hexdigest(),
                                   "saved_slot_names_at_time": slots})
                    state["frames"].append(record)
                    for name in slots:
                        state["saved_slots"][name] = {"sample_index": record["sample_index"],
                                                      "png_sha256": record["png_sha256"],
                                                      "pixel_sha256": record["pixel_sha256"]}
                    state["status"] = "capturing"
                    save_status()
                except Exception as error:
                    state["errors"].append(f"Frame processing failed: {type(error).__name__}: {error}")
                    state["status"] = "failed"
                    save_status()
                    capture_control.stop()
                    closed.set()

        @capture.event
        def on_closed():
            with lock:
                state["capture_item_closed_callback"] = True
                save_status()
            closed.set()

        state["status"] = "starting_native_session"
        save_status()
        control = capture.start_free_threaded()
        state["native_session_start_succeeded"] = True
        state["status"] = "waiting_for_frames"
        save_status()
        while not stop_event.is_set() and not closed.is_set() and time.monotonic() - started < seconds:
            if control.is_finished():
                state["native_thread_finished_before_deadline"] = True
                break
            time.sleep(0.05)
    except Exception as error:
        state["errors"].append(f"Capture failed: {type(error).__name__}: {error}")
        state["status"] = "failed"
    finally:
        # Persist before native shutdown, so a blocked native join remains diagnosable.
        state["cleanup_requested"] = True
        save_status()
        if control is not None:
            try:
                control.stop()
                state["cleanup_complete"] = True
            except Exception as error:
                state["errors"].append(f"Native shutdown failed: {type(error).__name__}: {error}")
        else:
            state["cleanup_complete"] = True
        if not state["errors"]:
            state["status"] = "completed" if diagnostics.sampled_frames else "no_frames"
        if not diagnostics.sampled_frames and not state["errors"]:
            state["failure_reason"] = "Native session yielded no saved frame during this interval"
        state["finished_utc"] = utc_now()
        save_status()
