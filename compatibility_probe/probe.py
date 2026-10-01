"""Run a bounded WGC plus click-through overlay compatibility check."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import math
import multiprocessing
import os
from pathlib import Path
import platform
import sys
import time
import uuid

from capture_worker import atomic_json, capture_worker, utc_now
from probe_logic import DEFAULT_BOARD_RECT, content_pixels, validate_board_rect, validate_content_aspect


PROBE_ROOT = Path(__file__).resolve().parent


def parse_hwnd(value: str) -> int:
    try:
        parsed = int(value, 0) if value.lower().startswith("0x") else int(value, 10)
    except ValueError as error:
        raise argparse.ArgumentTypeError("HWND must be decimal or 0x-prefixed hexadecimal") from error
    if parsed <= 0:
        raise argparse.ArgumentTypeError("HWND must be positive")
    return parsed


def parse_rect(value: str) -> tuple[float, float, float, float]:
    try:
        return validate_board_rect(tuple(float(part.strip()) for part in value.split(",")))
    except ValueError as error:
        raise argparse.ArgumentTypeError(str(error)) from error


def parse_aspect(value: str) -> tuple[float, float]:
    try:
        return validate_content_aspect(tuple(float(part.strip()) for part in value.split(":")))
    except ValueError as error:
        raise argparse.ArgumentTypeError(str(error)) from error


def positive_finite(value: str) -> float:
    parsed = float(value)
    if not math.isfinite(parsed) or parsed <= 0:
        raise argparse.ArgumentTypeError("value must be finite and positive")
    return parsed


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description="WGC + Win32 overlay compatibility probe; no probability calculation")
    result.add_argument("--list-windows", action="store_true", help="Print visible titled windows as JSON; no capture or overlay")
    result.add_argument("--hwnd", type=parse_hwnd, help="Exact HWND from --list-windows; no title substring matching")
    modes = result.add_mutually_exclusive_group()
    modes.add_argument("--capture-only", action="store_true", help="Run only the isolated WGC capture worker")
    modes.add_argument("--overlay-only", action="store_true", help="Run only the transparent 9x5 compatibility grid")
    result.add_argument("--seconds", type=positive_finite, default=45.0, help="Run duration, 0 < seconds <= 300 (default 45)")
    result.add_argument("--fps", type=positive_finite, default=5.0, help="Maximum PNG/diagnostic sampling rate, <=5 (default 5)")
    result.add_argument("--max-images", type=int, default=6, help="Retained PNG cap including first/latest, 2..20 (default 6)")
    result.add_argument("--start-delay", type=float, default=3.0, help="Seconds to manually focus the target before starting, 0..30 (default 3)")
    result.add_argument("--board-rect", type=parse_rect, default=DEFAULT_BOARD_RECT, help="Normalized x,y,width,height in the client or calibrated content (default .475,.270,.485,.480)")
    result.add_argument("--content-aspect", type=parse_aspect, default=None,
                        help="Optional manual MuMu calibration, e.g. 16:9; aspect-fit content is centered horizontally and bottom aligned")
    result.add_argument("--print-evidence", action="store_true", help="Print the complete evidence JSON instead of its concise summary")
    return result


def system_evidence(api) -> dict:
    version = sys.getwindowsversion()
    return {
        "platform": platform.platform(), "windows_version": {"major": version.major,
        "minor": version.minor, "build": version.build, "service_pack": version.service_pack},
        "python": sys.version, "python_executable": sys.executable,
        "process_bits": 64 if sys.maxsize > 2**32 else 32, "dpi_awareness_request": api.dpi_awareness,
        "virtual_screen": {"x": api.user.GetSystemMetrics(76), "y": api.user.GetSystemMetrics(77),
                           "width": api.user.GetSystemMetrics(78), "height": api.user.GetSystemMetrics(79)},
    }


def read_capture_status(path: Path) -> dict:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (FileNotFoundError, json.JSONDecodeError, OSError) as error:
        return {"status": "status_unavailable", "errors": [f"{type(error).__name__}: {error}"]}


def content_geometry_evidence(client: dict, aspect: tuple[float, float] | None) -> dict:
    if client["width"] <= 0 or client["height"] <= 0:
        return {"available": False, "reason": "target client has no positive size"}
    x, y, width, height = content_pixels(client["width"], client["height"], aspect)
    return {
        "source_content_client_rect": {"x": x, "y": y, "width": width, "height": height},
        "source_content_screen_rect": {"x": client["x"] + x, "y": client["y"] + y,
                                        "width": width, "height": height},
    }


def run(args, api) -> tuple[dict, Path]:
    from win32_overlay import Overlay

    windows = api.visible_windows()
    selected = next((item for item in windows if item["hwnd"] == args.hwnd), None)
    if selected is None:
        raise RuntimeError(f"HWND 0x{args.hwnd:X} is not a currently visible titled top-level window; list again")
    run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "_" + uuid.uuid4().hex[:8]
    directory = PROBE_ROOT / "artifacts" / run_id
    directory.mkdir(parents=True, exist_ok=False)
    capture_enabled = not args.overlay_only
    overlay_enabled = not args.capture_only
    evidence = {
        "schema_version": 1, "purpose": "native_capture_and_overlay_compatibility_only",
        "started_utc": utc_now(), "system": system_evidence(api), "target_initial": selected,
        "configuration": {"seconds": args.seconds, "max_save_fps": args.fps,
                          "max_images": args.max_images, "board_normalized_rect": list(args.board_rect),
                          "content_aspect": list(args.content_aspect) if args.content_aspect else None,
                          "content_layout_assumption": "manual_mumu_aspect_fit_center_bottom" if args.content_aspect else "entire_client_area",
                          "content_geometry_automatically_detected": False,
                          "capture": capture_enabled, "overlay": overlay_enabled},
        "boundaries": {"game_memory_read": False, "hooks_installed": False,
                       "input_events_generated": False, "probability_calculation": False,
                       "real_input_passthrough_tested": False, "true_exclusive_fullscreen_tested": False},
        "errors": [], "run_directory": str(directory),
    }
    evidence["content_geometry_initial"] = content_geometry_evidence(selected["client_screen_rect"], args.content_aspect)
    atomic_json(directory / "evidence.json", evidence)
    overlay = None
    process = None
    stop_event = None
    started = time.monotonic()
    evidence["stop_reason"] = "duration_elapsed"
    try:
        if args.start_delay:
            print(f"Starting in {args.start_delay:g}s; manually focus HWND 0x{args.hwnd:X}. Ctrl+C stops this probe.", file=sys.stderr, flush=True)
            time.sleep(args.start_delay)
        started = time.monotonic()
        if overlay_enabled:
            overlay = Overlay(api, args.hwnd, args.board_rect, args.content_aspect)
        if capture_enabled:
            context = multiprocessing.get_context("spawn")
            stop_event = context.Event()
            process = context.Process(target=capture_worker, args=(args.hwnd, str(directory), args.seconds,
                                     args.fps, args.max_images, stop_event), daemon=True, name="BAWGCCompatibilityWorker")
            process.start()
            evidence["capture_worker_pid"] = process.pid
        next_poll = started
        while time.monotonic() - started < args.seconds:
            if not api.user.IsWindow(args.hwnd):
                evidence["stop_reason"] = "target_window_closed"
                break
            if overlay:
                overlay.pump()
                if overlay.closed:
                    evidence["stop_reason"] = "overlay_window_closed"
                    break
                if time.monotonic() >= next_poll:
                    overlay.poll()
                    next_poll = time.monotonic() + 0.1
            if process and not process.is_alive():
                status = read_capture_status(directory / "capture_status.json")
                if status.get("errors") or status.get("status") in ("no_frames", "status_unavailable"):
                    evidence["stop_reason"] = "capture_worker_failed_or_yielded_no_frames"
                    break
                if not overlay:
                    evidence["stop_reason"] = "capture_worker_finished"
                    break
            time.sleep(0.01 if overlay else 0.05)
    except KeyboardInterrupt:
        evidence["stop_reason"] = "console_ctrl_c"
    except Exception as error:
        evidence["errors"].append(f"Probe failed: {type(error).__name__}: {error}")
        evidence["stop_reason"] = "probe_error"
    finally:
        if stop_event:
            stop_event.set()
        if overlay:
            overlay.close()
            evidence["overlay"] = overlay.evidence
        elif overlay_enabled:
            evidence["overlay"] = {"requested": True, "status": "not_created_or_failed",
                                   "real_input_passthrough_tested": False}
        else:
            evidence["overlay"] = {"requested": False}
        if process:
            process.join(timeout=3.0)
            forced = process.is_alive()
            if forced:
                process.terminate()  # Only the probe's owned child process, never the target.
                process.join(timeout=3.0)
            evidence["capture"] = read_capture_status(directory / "capture_status.json")
            evidence["capture"].update({"worker_exitcode": process.exitcode,
                                         "forced_child_termination": forced,
                                         "worker_still_alive": process.is_alive()})
            if forced:
                evidence["capture"]["cleanup_complete"] = False
                evidence["capture"]["errors"].append("Native startup/shutdown did not finish within the supervisor stop grace period; owned worker was terminated")
            if process.exitcode not in (0, None) and not forced:
                evidence["capture"]["errors"].append(f"Capture worker exited with code {process.exitcode}")
            process.close()
        else:
            evidence["capture"] = {"requested": capture_enabled, "status": "not_started"}
        try:
            evidence["target_final"] = api.window_info(args.hwnd)
            evidence["content_geometry_final"] = content_geometry_evidence(evidence["target_final"]["client_screen_rect"], args.content_aspect)
        except (OSError, RuntimeError) as error:
            evidence["target_final"] = {"available": False, "reason": str(error)}
        evidence["elapsed_seconds"] = round(time.monotonic() - started, 3)
        evidence["finished_utc"] = utc_now()
        evidence["retained_png_files"] = [str(path.name) for path in sorted(directory.glob("*.png"))]
        evidence["checks"] = {
            "capture_has_nonblack_sample": evidence["capture"].get("successful_sampled_frames", 0) > evidence["capture"].get("black_frame_candidates", 0) if capture_enabled else None,
            "overlay_static_styles_present": all(evidence["overlay"].get(key, False) for key in
                ("layered_style_set", "transparent_style_set", "noactivate_style_set", "topmost_style_set")) if overlay_enabled else None,
            "overlay_observed_visible": evidence["overlay"].get("show_transitions", 0) > 0 if overlay_enabled else None,
            "real_input_passthrough_tested": False,
            "true_exclusive_fullscreen_tested": False,
            "manual_acceptance_required": True,
        }
        atomic_json(directory / "evidence.json", evidence)
    return evidence, directory / "evidence.json"


def main() -> int:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
        sys.stderr.reconfigure(encoding="utf-8")
    arguments = parser()
    args = arguments.parse_args()
    if args.seconds > 300 or args.fps > 5 or not 2 <= args.max_images <= 20:
        arguments.error("seconds must be <=300, fps <=5, and max-images 2..20")
    if not math.isfinite(args.start_delay) or not 0 <= args.start_delay <= 30:
        arguments.error("start-delay must be finite and between 0 and 30")
    if not args.list_windows and args.hwnd is None:
        arguments.error("choose --list-windows or supply the exact --hwnd")
    try:
        from win32_overlay import Win32
        api = Win32()
        if args.list_windows:
            print(json.dumps({"system": system_evidence(api), "windows": api.visible_windows()},
                             ensure_ascii=False, indent=2))
            return 0
        evidence, evidence_path = run(args, api)
        capture = evidence["capture"]
        summary = {"evidence_file": str(evidence_path), "stop_reason": evidence["stop_reason"],
                   "checks": evidence["checks"], "capture_status": capture.get("status"),
                   "successful_sampled_frames": capture.get("successful_sampled_frames", 0),
                   "changed_frames": capture.get("changed_frames", 0),
                   "changed_frame_comparisons": capture.get("changed_frame_comparisons", 0),
                   "black_frame_candidates": capture.get("black_frame_candidates", 0),
                   "errors": evidence["errors"] + capture.get("errors", [])}
        print(json.dumps(evidence if args.print_evidence else summary, ensure_ascii=False, indent=2))
        if summary["errors"]:
            return 1
        if not args.overlay_only and not evidence["checks"]["capture_has_nonblack_sample"]:
            return 2
        return 0
    except Exception as error:
        print(json.dumps({"status": "failed", "reason": f"{type(error).__name__}: {error}"},
                         ensure_ascii=False, indent=2))
        return 1


if __name__ == "__main__":
    multiprocessing.freeze_support()
    raise SystemExit(main())
