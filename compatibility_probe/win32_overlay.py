"""Win32 metadata and an owned, non-activating color-key overlay.

No hooks, process-memory access, input synthesis, or game-window mutations.
"""

from __future__ import annotations

import ctypes
from ctypes import wintypes
import os
import time

from probe_logic import board_pixels, content_pixels, grid_lines


WS_EX_TOPMOST = 0x00000008
WS_EX_TRANSPARENT = 0x00000020
WS_EX_TOOLWINDOW = 0x00000080
WS_EX_LAYERED = 0x00080000
WS_EX_NOACTIVATE = 0x08000000
OVERLAY_EX_STYLE = WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE
GWL_STYLE = -16
GWL_EXSTYLE = -20
SW_HIDE = 0
SW_SHOWNOACTIVATE = 4
SWP_NOACTIVATE = 0x0010
SWP_SHOWWINDOW = 0x0040
WM_DESTROY = 0x0002
WM_PAINT = 0x000F
WM_CLOSE = 0x0010
WM_ERASEBKGND = 0x0014
WM_MOUSEACTIVATE = 0x0021
WM_NCHITTEST = 0x0084
WM_QUIT = 0x0012
COLOR_KEY = 0x00030201
GRID_COLOR = 0x00DCEB46


class Win32:
    def __init__(self) -> None:
        if os.name != "nt":
            raise RuntimeError("This probe needs Windows 10/11 x64")
        self.user = ctypes.WinDLL("user32", use_last_error=True)
        self.gdi = ctypes.WinDLL("gdi32", use_last_error=True)
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.dwm = ctypes.WinDLL("dwmapi", use_last_error=True)
        self.LRESULT = ctypes.c_ssize_t
        self.WPARAM = ctypes.c_size_t
        self.LPARAM = ctypes.c_ssize_t
        self.WNDPROC = ctypes.WINFUNCTYPE(self.LRESULT, wintypes.HWND, wintypes.UINT, self.WPARAM, self.LPARAM)
        self.ENUMPROC = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, self.LPARAM)
        self._bind()
        self.dpi_awareness = self._set_dpi_awareness()

    @staticmethod
    def check(value, name: str):
        if not value:
            raise ctypes.WinError(ctypes.get_last_error(), name)
        return value

    def _function(self, library, name, arguments, result):
        function = getattr(library, name)
        function.argtypes = arguments
        function.restype = result
        return function

    def _bind(self) -> None:
        f = self._function
        u, g, k = self.user, self.gdi, self.kernel
        f(u, "EnumWindows", [self.ENUMPROC, self.LPARAM], wintypes.BOOL)
        for name in ("IsWindow", "IsWindowVisible", "IsIconic"):
            f(u, name, [wintypes.HWND], wintypes.BOOL)
        f(u, "GetWindowTextLengthW", [wintypes.HWND], ctypes.c_int)
        f(u, "GetWindowTextW", [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int], ctypes.c_int)
        f(u, "GetClassNameW", [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int], ctypes.c_int)
        f(u, "GetWindowThreadProcessId", [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)], wintypes.DWORD)
        self.get_long = f(u, "GetWindowLongPtrW", [wintypes.HWND, ctypes.c_int], ctypes.c_ssize_t)
        f(u, "GetWindowRect", [wintypes.HWND, ctypes.POINTER(wintypes.RECT)], wintypes.BOOL)
        f(u, "GetClientRect", [wintypes.HWND, ctypes.POINTER(wintypes.RECT)], wintypes.BOOL)
        f(u, "ClientToScreen", [wintypes.HWND, ctypes.POINTER(wintypes.POINT)], wintypes.BOOL)
        f(u, "GetForegroundWindow", [], wintypes.HWND)
        f(u, "GetAncestor", [wintypes.HWND, wintypes.UINT], wintypes.HWND)
        f(u, "MonitorFromWindow", [wintypes.HWND, wintypes.DWORD], wintypes.HANDLE)
        f(u, "GetMonitorInfoW", [wintypes.HANDLE, ctypes.c_void_p], wintypes.BOOL)
        f(u, "GetSystemMetrics", [ctypes.c_int], ctypes.c_int)
        f(u, "SetProcessDpiAwarenessContext", [ctypes.c_void_p], wintypes.BOOL)
        f(u, "SetProcessDPIAware", [], wintypes.BOOL)
        f(k, "GetModuleHandleW", [wintypes.LPCWSTR], wintypes.HMODULE)
        f(u, "RegisterClassExW", [ctypes.c_void_p], wintypes.ATOM)
        f(u, "UnregisterClassW", [wintypes.LPCWSTR, wintypes.HINSTANCE], wintypes.BOOL)
        f(u, "CreateWindowExW", [wintypes.DWORD, wintypes.LPCWSTR, wintypes.LPCWSTR, wintypes.DWORD,
                               ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
                               wintypes.HWND, wintypes.HMENU, wintypes.HINSTANCE, ctypes.c_void_p], wintypes.HWND)
        f(u, "DefWindowProcW", [wintypes.HWND, wintypes.UINT, self.WPARAM, self.LPARAM], self.LRESULT)
        f(u, "SetLayeredWindowAttributes", [wintypes.HWND, wintypes.COLORREF, wintypes.BYTE, wintypes.DWORD], wintypes.BOOL)
        f(u, "SetWindowPos", [wintypes.HWND, wintypes.HWND, ctypes.c_int, ctypes.c_int, ctypes.c_int,
                             ctypes.c_int, wintypes.UINT], wintypes.BOOL)
        f(u, "ShowWindow", [wintypes.HWND, ctypes.c_int], wintypes.BOOL)
        f(u, "InvalidateRect", [wintypes.HWND, ctypes.c_void_p, wintypes.BOOL], wintypes.BOOL)
        f(u, "DestroyWindow", [wintypes.HWND], wintypes.BOOL)
        f(u, "PostQuitMessage", [ctypes.c_int], None)
        f(u, "PeekMessageW", [ctypes.POINTER(wintypes.MSG), wintypes.HWND, wintypes.UINT,
                              wintypes.UINT, wintypes.UINT], wintypes.BOOL)
        f(u, "TranslateMessage", [ctypes.POINTER(wintypes.MSG)], wintypes.BOOL)
        f(u, "DispatchMessageW", [ctypes.POINTER(wintypes.MSG)], self.LRESULT)
        f(u, "BeginPaint", [wintypes.HWND, ctypes.c_void_p], wintypes.HDC)
        f(u, "EndPaint", [wintypes.HWND, ctypes.c_void_p], wintypes.BOOL)
        f(u, "FillRect", [wintypes.HDC, ctypes.POINTER(wintypes.RECT), wintypes.HBRUSH], ctypes.c_int)
        f(g, "CreateSolidBrush", [wintypes.COLORREF], wintypes.HBRUSH)
        f(g, "CreatePen", [ctypes.c_int, ctypes.c_int, wintypes.COLORREF], wintypes.HANDLE)
        f(g, "SelectObject", [wintypes.HDC, wintypes.HANDLE], wintypes.HANDLE)
        f(g, "DeleteObject", [wintypes.HANDLE], wintypes.BOOL)
        f(g, "MoveToEx", [wintypes.HDC, ctypes.c_int, ctypes.c_int, ctypes.c_void_p], wintypes.BOOL)
        f(g, "LineTo", [wintypes.HDC, ctypes.c_int, ctypes.c_int], wintypes.BOOL)
        f(g, "SetBkMode", [wintypes.HDC, ctypes.c_int], ctypes.c_int)
        f(g, "SetTextColor", [wintypes.HDC, wintypes.COLORREF], wintypes.COLORREF)
        f(g, "TextOutW", [wintypes.HDC, ctypes.c_int, ctypes.c_int, wintypes.LPCWSTR, ctypes.c_int], wintypes.BOOL)
        f(self.dwm, "DwmGetWindowAttribute", [wintypes.HWND, wintypes.DWORD, ctypes.c_void_p,
                                            wintypes.DWORD], ctypes.c_long)
        f(u, "SetWindowDisplayAffinity", [wintypes.HWND, wintypes.DWORD], wintypes.BOOL)

    def _set_dpi_awareness(self) -> dict:
        if self.user.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4)):
            return {"requested": "per_monitor_v2", "request_succeeded": True}
        error = ctypes.get_last_error()
        fallback = bool(self.user.SetProcessDPIAware())
        return {"requested": "per_monitor_v2", "request_succeeded": False,
                "win32_error": error, "fallback_system_aware_requested": fallback}

    @staticmethod
    def _rect(rect) -> dict:
        return {"x": rect.left, "y": rect.top, "width": rect.right - rect.left,
                "height": rect.bottom - rect.top}

    def geometry(self, hwnd: int) -> dict:
        if not self.user.IsWindow(hwnd):
            raise RuntimeError(f"Target HWND 0x{hwnd:X} no longer exists")
        window = wintypes.RECT()
        client = wintypes.RECT()
        self.check(self.user.GetWindowRect(hwnd, ctypes.byref(window)), "GetWindowRect")
        self.check(self.user.GetClientRect(hwnd, ctypes.byref(client)), "GetClientRect")
        origin = wintypes.POINT(client.left, client.top)
        end = wintypes.POINT(client.right, client.bottom)
        self.check(self.user.ClientToScreen(hwnd, ctypes.byref(origin)), "ClientToScreen(origin)")
        self.check(self.user.ClientToScreen(hwnd, ctypes.byref(end)), "ClientToScreen(end)")
        client_screen = {"x": origin.x, "y": origin.y, "width": end.x - origin.x, "height": end.y - origin.y}

        class MONITORINFO(ctypes.Structure):
            _fields_ = [("cbSize", wintypes.DWORD), ("rcMonitor", wintypes.RECT),
                        ("rcWork", wintypes.RECT), ("dwFlags", wintypes.DWORD)]

        monitor = MONITORINFO()
        monitor.cbSize = ctypes.sizeof(monitor)
        handle = self.user.MonitorFromWindow(hwnd, 2)
        self.check(self.user.GetMonitorInfoW(handle, ctypes.byref(monitor)), "GetMonitorInfoW")
        frame = wintypes.RECT()
        frame_result = self.dwm.DwmGetWindowAttribute(hwnd, 9, ctypes.byref(frame), ctypes.sizeof(frame))
        cloaked = wintypes.DWORD()
        cloak_result = self.dwm.DwmGetWindowAttribute(hwnd, 14, ctypes.byref(cloaked), ctypes.sizeof(cloaked))
        monitor_rect = self._rect(monitor.rcMonitor)
        return {
            "window_rect": self._rect(window), "client_screen_rect": client_screen,
            "dwm_extended_frame_rect": self._rect(frame) if frame_result == 0 else None,
            "monitor_rect": monitor_rect, "monitor_work_rect": self._rect(monitor.rcWork),
            "visible": bool(self.user.IsWindowVisible(hwnd)), "minimized": bool(self.user.IsIconic(hwnd)),
            "cloaked": bool(cloaked.value) if cloak_result == 0 else None,
            "foreground": self.user.GetForegroundWindow() == hwnd,
            "style_hex": f"0x{self.get_long(hwnd, GWL_STYLE) & 0xFFFFFFFF:08X}",
            "extended_style_hex": f"0x{self.get_long(hwnd, GWL_EXSTYLE) & 0xFFFFFFFF:08X}",
            "window_rect_matches_monitor": self._rect(window) == monitor_rect,
            "fullscreen_rendering_mode": "unknown_geometry_does_not_prove_exclusive_fullscreen",
        }

    def window_info(self, hwnd: int) -> dict:
        length = self.user.GetWindowTextLengthW(hwnd)
        title = ctypes.create_unicode_buffer(length + 1)
        self.user.GetWindowTextW(hwnd, title, len(title))
        class_name = ctypes.create_unicode_buffer(256)
        self.user.GetClassNameW(hwnd, class_name, len(class_name))
        pid = wintypes.DWORD()
        thread = self.user.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        return {"hwnd": hwnd, "hwnd_hex": f"0x{hwnd:X}", "title": title.value,
                "class_name": class_name.value, "pid": pid.value, "thread_id": thread,
                **self.geometry(hwnd)}

    def visible_windows(self) -> list[dict]:
        windows = []

        @self.ENUMPROC
        def callback(hwnd, _):
            if self.user.IsWindowVisible(hwnd) and self.user.GetWindowTextLengthW(hwnd) > 0:
                try:
                    windows.append(self.window_info(int(hwnd)))
                except (OSError, RuntimeError):
                    pass  # The window may close during enumeration.
            return True

        self.check(self.user.EnumWindows(callback, 0), "EnumWindows")
        return windows


class Overlay:
    def __init__(self, api: Win32, target_hwnd: int, normalized: tuple[float, float, float, float],
                 content_aspect: tuple[float, float] | None = None) -> None:
        self.api = api
        self.target = target_hwnd
        self.normalized = normalized
        self.content_aspect = content_aspect
        self.hwnd = None
        self.class_name = f"BACompatibilityProbe_{os.getpid()}"
        self.module = api.kernel.GetModuleHandleW(None)
        self.width = self.height = 1
        self.visible = False
        self.closed = False
        self.last_geometry = None
        self.callback_error = None
        self.brush = self.pen = None
        self.class_registered = False
        self.evidence = {
            "requested": True, "grid": {"columns": 9, "rows": 5},
            "board_normalized_rect": list(normalized), "probability_computation": False,
            "content_aspect": list(content_aspect) if content_aspect else None,
            "content_layout_assumption": "manual_mumu_aspect_fit_center_bottom" if content_aspect else "entire_client_area",
            "content_geometry_automatically_detected": False,
            "input_passthrough_verification": "static_window_attributes_only",
            "real_input_passthrough_tested": False, "true_exclusive_fullscreen_tested": False,
            "show_transitions": 0, "hide_transitions": 0, "geometry_updates": 0,
            "observed_visible_seconds": 0.0, "cleanup_complete": False,
        }
        self.last_poll = time.monotonic()
        self.callback = api.WNDPROC(self._wndproc)
        try:
            self._create()
        except BaseException:
            self.close()
            raise

    def _create(self) -> None:
        api = self.api

        class WNDCLASSEXW(ctypes.Structure):
            _fields_ = [("cbSize", wintypes.UINT), ("style", wintypes.UINT), ("lpfnWndProc", api.WNDPROC),
                        ("cbClsExtra", ctypes.c_int), ("cbWndExtra", ctypes.c_int),
                        ("hInstance", wintypes.HINSTANCE), ("hIcon", wintypes.HICON),
                        ("hCursor", wintypes.HANDLE), ("hbrBackground", wintypes.HBRUSH),
                        ("lpszMenuName", wintypes.LPCWSTR), ("lpszClassName", wintypes.LPCWSTR),
                        ("hIconSm", wintypes.HICON)]

        wc = WNDCLASSEXW()
        wc.cbSize = ctypes.sizeof(wc)
        wc.lpfnWndProc = self.callback
        wc.hInstance = self.module
        wc.lpszClassName = self.class_name
        api.check(api.user.RegisterClassExW(ctypes.byref(wc)), "RegisterClassExW")
        self.class_registered = True
        self.brush = api.check(api.gdi.CreateSolidBrush(COLOR_KEY), "CreateSolidBrush")
        self.pen = api.check(api.gdi.CreatePen(0, 1, GRID_COLOR), "CreatePen")
        self.hwnd = api.check(api.user.CreateWindowExW(
            OVERLAY_EX_STYLE, self.class_name, "BA capture compatibility test - no probabilities", 0x80000000,
            0, 0, 1, 1, None, None, self.module, None), "CreateWindowExW")
        api.check(api.user.SetLayeredWindowAttributes(self.hwnd, COLOR_KEY, 255, 1),
                  "SetLayeredWindowAttributes")
        ctypes.set_last_error(0)
        excluded = bool(api.user.SetWindowDisplayAffinity(self.hwnd, 0x11))
        ex_style = api.get_long(self.hwnd, GWL_EXSTYLE) & 0xFFFFFFFF
        self.evidence.update({
            "hwnd_hex": f"0x{self.hwnd:X}", "actual_extended_style_hex": f"0x{ex_style:08X}",
            "layered_style_set": bool(ex_style & WS_EX_LAYERED),
            "transparent_style_set": bool(ex_style & WS_EX_TRANSPARENT),
            "noactivate_style_set": bool(ex_style & WS_EX_NOACTIVATE),
            "topmost_style_set": bool(ex_style & WS_EX_TOPMOST),
            "color_key_transparency_call_succeeded": True,
            "capture_exclusion_request_succeeded": excluded,
            "capture_exclusion_request_win32_error": None if excluded else ctypes.get_last_error(),
            "nchittest_response": "HTTRANSPARENT", "mouseactivate_response": "MA_NOACTIVATE",
        })
        self.poll()

    def _wndproc(self, hwnd, message, wparam, lparam):
        try:
            if message == WM_NCHITTEST:
                return -1  # HTTRANSPARENT; style remains the primary cross-thread protection.
            if message == WM_MOUSEACTIVATE:
                return 3  # MA_NOACTIVATE
            if message == WM_ERASEBKGND:
                return 1
            if message == WM_PAINT:
                self._paint(hwnd)
                return 0
            if message == WM_CLOSE:
                self.api.user.DestroyWindow(hwnd)
                return 0
            if message == WM_DESTROY:
                self.closed = True
                self.api.user.PostQuitMessage(0)
                return 0
        except BaseException as error:
            self.callback_error = f"{type(error).__name__}: {error}"
        return self.api.user.DefWindowProcW(hwnd, message, wparam, lparam)

    def _paint(self, hwnd) -> None:
        api = self.api

        class PAINTSTRUCT(ctypes.Structure):
            _fields_ = [("hdc", wintypes.HDC), ("fErase", wintypes.BOOL), ("rcPaint", wintypes.RECT),
                        ("fRestore", wintypes.BOOL), ("fIncUpdate", wintypes.BOOL),
                        ("rgbReserved", wintypes.BYTE * 32)]

        paint = PAINTSTRUCT()
        hdc = api.user.BeginPaint(hwnd, ctypes.byref(paint))
        if not hdc:
            return
        try:
            rect = wintypes.RECT(0, 0, self.width, self.height)
            api.user.FillRect(hdc, ctypes.byref(rect), self.brush)
            if self.width < 4 or self.height < 4:
                return
            board = board_pixels(self.width, self.height, self.normalized, self.content_aspect)
            vertical, horizontal = grid_lines(board)
            previous = api.gdi.SelectObject(hdc, self.pen)
            try:
                for x in vertical:
                    api.gdi.MoveToEx(hdc, x, board[1], None)
                    api.gdi.LineTo(hdc, x, board[3] + 1)
                for y in horizontal:
                    api.gdi.MoveToEx(hdc, board[0], y, None)
                    api.gdi.LineTo(hdc, board[2] + 1, y)
            finally:
                api.gdi.SelectObject(hdc, previous)
            api.gdi.SetBkMode(hdc, 1)
            api.gdi.SetTextColor(hdc, GRID_COLOR)
            text = "COMPATIBILITY TEST - NO PROBABILITIES (9 x 5)"
            api.gdi.TextOutW(hdc, board[0], max(0, board[1] - 22), text, len(text))
        finally:
            api.user.EndPaint(hwnd, ctypes.byref(paint))

    def poll(self) -> None:
        if self.closed or not self.hwnd:
            return
        now = time.monotonic()
        if self.visible:
            self.evidence["observed_visible_seconds"] += now - self.last_poll
        self.last_poll = now
        geometry = self.api.geometry(self.target)
        client = geometry["client_screen_rect"]
        foreground = self.api.user.GetForegroundWindow()
        root = self.api.user.GetAncestor(foreground, 2) if foreground else None
        should_show = (foreground == self.target or root == self.target) and geometry["visible"] and not geometry["minimized"] and not geometry["cloaked"] and client["width"] > 0 and client["height"] > 0
        if should_show:
            if client != self.last_geometry:
                self.width, self.height = client["width"], client["height"]
                self.api.check(self.api.user.SetWindowPos(
                    self.hwnd, wintypes.HWND(-1), client["x"], client["y"], self.width, self.height,
                    SWP_NOACTIVATE), "SetWindowPos")
                self.api.user.InvalidateRect(self.hwnd, None, True)
                self.last_geometry = client.copy()
                self.evidence["geometry_updates"] += 1
                self.evidence["last_client_screen_rect"] = client.copy()
                content_x, content_y, content_width, content_height = content_pixels(self.width, self.height, self.content_aspect)
                self.evidence["last_source_content_client_rect"] = {"x": content_x, "y": content_y,
                    "width": content_width, "height": content_height}
                self.evidence["last_source_content_screen_rect"] = {"x": client["x"] + content_x,
                    "y": client["y"] + content_y, "width": content_width, "height": content_height}
                self.evidence["last_board_client_rect"] = list(board_pixels(self.width, self.height, self.normalized, self.content_aspect))
            if not self.visible:
                self.api.user.ShowWindow(self.hwnd, SW_SHOWNOACTIVATE)
                self.visible = True
                self.evidence["show_transitions"] += 1
        elif self.visible:
            self.api.user.ShowWindow(self.hwnd, SW_HIDE)
            self.visible = False
            self.evidence["hide_transitions"] += 1

    def pump(self) -> None:
        message = wintypes.MSG()
        while self.api.user.PeekMessageW(ctypes.byref(message), None, 0, 0, 1):
            if message.message == WM_QUIT:
                self.closed = True
                break
            self.api.user.TranslateMessage(ctypes.byref(message))
            self.api.user.DispatchMessageW(ctypes.byref(message))
        if self.callback_error:
            raise RuntimeError(f"Overlay callback error: {self.callback_error}")

    def close(self) -> None:
        if self.hwnd and self.api.user.IsWindow(self.hwnd):
            self.api.user.ShowWindow(self.hwnd, SW_HIDE)
            self.api.user.DestroyWindow(self.hwnd)
        if self.class_registered:
            self.api.user.UnregisterClassW(self.class_name, self.module)
            self.class_registered = False
        for handle in (self.pen, self.brush):
            if handle:
                self.api.gdi.DeleteObject(handle)
        self.pen = self.brush = None
        self.evidence["observed_visible_seconds"] = round(self.evidence["observed_visible_seconds"], 3)
        self.evidence["cleanup_complete"] = not self.hwnd or not self.api.user.IsWindow(self.hwnd)
        self.hwnd = None
