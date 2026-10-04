# Dev-only fake PokerStars tables for the e2e driver (scripts/sim/e2e.mjs).
# Never part of the app or its bundle; it never touches the real client.
#
# Opens top-level windows with the real table class (GLFW30) and
# PokerStars-like titles, paints a mock felt (seat plates, hole cards, board,
# action buttons) where the HUD's seat layout expects them, and counts left
# clicks per window into a JSON status file, so "the table stays clickable
# outside HUD elements" can be measured. Also captures screen regions and
# moves/clicks the mouse for the driver, all on one per-monitor DPI-aware
# thread so every coordinate is in physical pixels, like GetWindowRect.
#
# Windows PowerShell 5.1 and the C# compiler of .NET Framework (Add-Type):
# nothing to install.
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File fake-tables.ps1 -Probe
#       compiles the program and prints the monitors as JSON; creates no window
#   powershell -NoProfile -ExecutionPolicy Bypass -File fake-tables.ps1 -Status <file>
#       reads one JSON command per line on stdin, answers one JSON line each:
#       {"seq":1,"op":"open","id":"t1","title":"...","x":0,"y":0,"width":483,"height":359,"seats":6}
#       {"seq":2,"op":"move","id":"t1","x":10,"y":10,"width":800,"height":570}
#       {"seq":3,"op":"retitle","id":"t1","title":"..."}
#       {"seq":4,"op":"close","id":"t1"}
#       {"seq":5,"op":"shot","path":"C:\\...\\a.png","x":0,"y":0,"width":800,"height":600}
#       {"seq":5,"op":"print","id":"t1","path":"C:\...\t1.png","overlay":"C:\...\hud.png"}
#           renders the table's own window (works on a locked session), HUD PNG on top
#       {"seq":6,"op":"mouse","x":100,"y":100}   {"seq":7,"op":"click","x":100,"y":100}
#       {"seq":8,"op":"list","title":"Velora"}   top-level windows whose title contains it
#       {"seq":9,"op":"keys","ctrl":true,"alt":true,"vk":80}   presses Ctrl+Alt+P
#       {"seq":9,"op":"show","hwnd":123,"cmd":6}  ShowWindow (6 minimizes)
#       {"seq":9,"op":"status"}                  {"seq":10,"op":"quit"}

param(
  [string]$Status,
  [switch]$Probe
)

$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)

$source = @'
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

namespace VeloraSim
{
    public class FakeTables
    {
        public const string ClassName = "GLFW30";

        [StructLayout(LayoutKind.Sequential)] struct POINT { public int X; public int Y; }
        [StructLayout(LayoutKind.Sequential)] struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
        [StructLayout(LayoutKind.Sequential)]
        struct MSG { public IntPtr hwnd; public uint message; public IntPtr wParam; public IntPtr lParam; public uint time; public POINT pt; }
        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        struct WNDCLASSEX
        {
            public uint cbSize; public uint style; public WndProc lpfnWndProc; public int cbClsExtra; public int cbWndExtra;
            public IntPtr hInstance; public IntPtr hIcon; public IntPtr hCursor; public IntPtr hbrBackground;
            public string lpszMenuName; public string lpszClassName; public IntPtr hIconSm;
        }
        [StructLayout(LayoutKind.Sequential)]
        struct PAINTSTRUCT { public IntPtr hdc; public int fErase; public RECT rcPaint; public int fRestore; public int fIncUpdate; [MarshalAs(UnmanagedType.ByValArray, SizeConst = 32)] public byte[] rgbReserved; }
        [StructLayout(LayoutKind.Sequential)]
        struct MONITORINFO { public int cbSize; public RECT rcMonitor; public RECT rcWork; public uint dwFlags; }

        delegate IntPtr WndProc(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);
        delegate bool MonitorEnumProc(IntPtr hMonitor, IntPtr hdc, IntPtr lprc, IntPtr data);
        delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr data);

        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern ushort RegisterClassEx(ref WNDCLASSEX wc);
        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern IntPtr CreateWindowEx(uint exStyle, string cls, string title, uint style, int x, int y, int w, int h, IntPtr parent, IntPtr menu, IntPtr inst, IntPtr param);
        [DllImport("user32.dll")] static extern bool DestroyWindow(IntPtr hWnd);
        [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr hWnd, int cmd);
        [DllImport("user32.dll")] static extern bool UpdateWindow(IntPtr hWnd);
        [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr hWnd, IntPtr after, int x, int y, int w, int h, uint flags);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool SetWindowText(IntPtr hWnd, string text);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr hWnd, StringBuilder text, int max);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr hWnd, StringBuilder text, int max);
        [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hWnd);
        [DllImport("user32.dll")] static extern bool IsWindow(IntPtr hWnd);
        [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
        [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr hWnd, out RECT rect);
        [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr hWnd, ref POINT p);
        [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
        [DllImport("user32.dll")] static extern bool EnumWindows(EnumWindowsProc proc, IntPtr data);
        // Unicode like the class and its windows: DefWindowProcA would store a
        // Unicode title as ANSI, which other processes read as "S".
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr DefWindowProc(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowTextLength(IntPtr hWnd);
        [DllImport("user32.dll")] static extern int GetMessage(out MSG msg, IntPtr hWnd, uint min, uint max);
        [DllImport("user32.dll")] static extern bool TranslateMessage(ref MSG msg);
        [DllImport("user32.dll")] static extern IntPtr DispatchMessage(ref MSG msg);
        [DllImport("user32.dll")] static extern bool PostThreadMessage(uint thread, uint msg, IntPtr wParam, IntPtr lParam);
        [DllImport("user32.dll")] static extern void PostQuitMessage(int code);
        [DllImport("user32.dll")] static extern IntPtr BeginPaint(IntPtr hWnd, out PAINTSTRUCT ps);
        [DllImport("user32.dll")] static extern bool EndPaint(IntPtr hWnd, ref PAINTSTRUCT ps);
        [DllImport("user32.dll")] static extern bool InvalidateRect(IntPtr hWnd, IntPtr rect, bool erase);
        [DllImport("user32.dll")] static extern IntPtr LoadCursor(IntPtr inst, IntPtr name);
        [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")] static extern bool GetCursorPos(out POINT p);
        [DllImport("user32.dll")] static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
        [DllImport("user32.dll")] static extern void mouse_event(uint flags, int dx, int dy, uint data, UIntPtr extra);
        [DllImport("user32.dll")] static extern bool EnumDisplayMonitors(IntPtr hdc, IntPtr clip, MonitorEnumProc proc, IntPtr data);
        [DllImport("user32.dll")] static extern bool GetMonitorInfo(IntPtr hMonitor, ref MONITORINFO info);
        [DllImport("user32.dll")] static extern bool SetProcessDPIAware();
        [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
        [DllImport("user32.dll")] static extern IntPtr GetDC(IntPtr hWnd);
        [DllImport("user32.dll")] static extern int ReleaseDC(IntPtr hWnd, IntPtr hdc);
        [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr hWnd, IntPtr hdc, uint flags);
        [DllImport("gdi32.dll", SetLastError = true)] static extern bool BitBlt(IntPtr dest, int x, int y, int w, int h, IntPtr src, int sx, int sy, uint rop);
        [DllImport("wtsapi32.dll", SetLastError = true)] static extern bool WTSQuerySessionInformation(IntPtr server, int session, int infoClass, out IntPtr buffer, out int bytes);
        [DllImport("wtsapi32.dll")] static extern void WTSFreeMemory(IntPtr memory);
        [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern IntPtr GetModuleHandle(string name);

        const uint WS_OVERLAPPEDWINDOW = 0x00CF0000;
        const uint WM_DESTROY = 0x0002, WM_PAINT = 0x000F, WM_CLOSE = 0x0010, WM_ERASEBKGND = 0x0014, WM_SIZE = 0x0005;
        const uint WM_LBUTTONDOWN = 0x0201, WM_APP = 0x8000;
        const uint SWP_NOZORDER = 0x0004, SWP_NOACTIVATE = 0x0010;
        const uint MOUSEEVENTF_LEFTDOWN = 0x0002, MOUSEEVENTF_LEFTUP = 0x0004;
        const uint SRCCOPY = 0x00CC0020, CAPTUREBLT = 0x40000000;
        const uint PW_RENDERFULLCONTENT = 2;
        static readonly IntPtr PER_MONITOR_AWARE_V2 = new IntPtr(-4);

        class Table
        {
            public string Id; public IntPtr Hwnd; public string Title; public int Seats; public int Clicks;
            public int LastClickX; public int LastClickY; public bool HasClick;
        }

        class Job
        {
            public Func<string> Work; public string Result; public Exception Error;
            public ManualResetEvent Done = new ManualResetEvent(false);
        }

        readonly string statusPath;
        readonly List<Table> tables = new List<Table>();
        readonly Queue<Job> jobs = new Queue<Job>();
        readonly ManualResetEvent ready = new ManualResetEvent(false);
        WndProc proc;
        Thread ui;
        uint uiThreadId;
        Exception startError;

        public FakeTables(string statusPath) { this.statusPath = statusPath; }

        static void BecomeDpiAware()
        {
            try { if (SetThreadDpiAwarenessContext(PER_MONITOR_AWARE_V2) != IntPtr.Zero) return; }
            catch (EntryPointNotFoundException) { }
            SetProcessDPIAware();
        }

        // ------------------------------------------------------------ probe

        /// "true" when this Windows session is locked (the lock screen covers
        /// every window and takes the mouse and keyboard), "false" when it is
        /// not, "null" when Windows does not say.
        public static string SessionLocked()
        {
            const int WTSSessionInfoEx = 25;
            IntPtr buffer;
            int bytes;
            if (!WTSQuerySessionInformation(IntPtr.Zero, -1, WTSSessionInfoEx, out buffer, out bytes)) return "null";
            try
            {
                // WTSINFOEX: Level (DWORD), then the 8-aligned WTSINFOEX_LEVEL1:
                // SessionId, SessionState, SessionFlags (0 locked, 1 unlocked).
                if (bytes < 20 || Marshal.ReadInt32(buffer, 0) != 1) return "null";
                int flags = Marshal.ReadInt32(buffer, 16);
                return flags == 0 ? "true" : flags == 1 ? "false" : "null";
            }
            finally
            {
                WTSFreeMemory(buffer);
            }
        }

        /// The title Windows reports for `hwnd`, as any other process (the
        /// HUD's table_track) reads it.
        static string ReadTitle(IntPtr hwnd)
        {
            StringBuilder sb = new StringBuilder(GetWindowTextLength(hwnd) + 2);
            GetWindowText(hwnd, sb, sb.Capacity);
            return sb.ToString();
        }

        /// A hidden window (never shown) of a class of its own, given a
        /// non-ASCII title and read back, then destroyed: "true" when the
        /// title survives, as the HUD must read the table titles whole.
        static string TitleRoundTrip()
        {
            const string title = "Session: 00:00 - Velora probe \u00e9\u20ac - Logged Out";
            WNDCLASSEX wc = new WNDCLASSEX();
            wc.cbSize = (uint)Marshal.SizeOf(typeof(WNDCLASSEX));
            WndProc probeProc = DefWindowProc;
            wc.lpfnWndProc = probeProc;
            wc.hInstance = GetModuleHandle(null);
            wc.lpszClassName = "VeloraSimProbe";
            if (RegisterClassEx(ref wc) == 0) return "false";
            IntPtr hwnd = CreateWindowEx(0, wc.lpszClassName, title, WS_OVERLAPPEDWINDOW, 0, 0, 100, 100, IntPtr.Zero, IntPtr.Zero, wc.hInstance, IntPtr.Zero);
            if (hwnd == IntPtr.Zero) return "false";
            try
            {
                return ReadTitle(hwnd) == title ? "true" : "false";
            }
            finally
            {
                DestroyWindow(hwnd);
                GC.KeepAlive(probeProc);
            }
        }

        /// Monitors (physical pixels) as JSON, from a DPI-aware thread; no
        /// window is shown.
        public static string Probe()
        {
            string result = null;
            Thread t = new Thread(delegate ()
            {
                BecomeDpiAware();
                StringBuilder sb = new StringBuilder();
                sb.Append("{\"ok\":true,\"className\":\"").Append(ClassName).Append("\",\"monitors\":[");
                bool first = true;
                EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero, delegate (IntPtr m, IntPtr hdc, IntPtr r, IntPtr d)
                {
                    MONITORINFO info = new MONITORINFO();
                    info.cbSize = Marshal.SizeOf(typeof(MONITORINFO));
                    if (GetMonitorInfo(m, ref info))
                    {
                        if (!first) sb.Append(",");
                        first = false;
                        sb.Append("{\"primary\":").Append((info.dwFlags & 1) != 0 ? "true" : "false");
                        sb.Append(",\"bounds\":").Append(RectJson(info.rcMonitor));
                        sb.Append(",\"workArea\":").Append(RectJson(info.rcWork)).Append("}");
                    }
                    return true;
                }, IntPtr.Zero);
                sb.Append("],\"locked\":").Append(SessionLocked());
                sb.Append(",\"titleRoundTrip\":").Append(TitleRoundTrip()).Append("}");
                result = sb.ToString();
            });
            t.Start();
            t.Join();
            return result;
        }

        // ------------------------------------------------------------ UI thread

        public void Start()
        {
            ui = new Thread(Loop);
            ui.IsBackground = true;
            ui.SetApartmentState(ApartmentState.STA);
            ui.Start();
            ready.WaitOne();
            if (startError != null) throw startError;
            WriteStatus();
        }

        void Loop()
        {
            try
            {
                BecomeDpiAware();
                uiThreadId = GetCurrentThreadId();
                proc = new WndProc(WindowProc);
                WNDCLASSEX wc = new WNDCLASSEX();
                wc.cbSize = (uint)Marshal.SizeOf(typeof(WNDCLASSEX));
                wc.style = 0x0003; // CS_HREDRAW | CS_VREDRAW
                wc.lpfnWndProc = proc;
                wc.hInstance = GetModuleHandle(null);
                wc.hCursor = LoadCursor(IntPtr.Zero, new IntPtr(32512));
                wc.lpszClassName = ClassName;
                if (RegisterClassEx(ref wc) == 0) throw new InvalidOperationException("RegisterClassEx failed: " + Marshal.GetLastWin32Error());
            }
            catch (Exception e)
            {
                startError = e;
                ready.Set();
                return;
            }
            MSG msg;
            // Create the thread's message queue before anyone posts to it.
            ready.Set();
            while (GetMessage(out msg, IntPtr.Zero, 0, 0) > 0)
            {
                if (msg.hwnd == IntPtr.Zero && msg.message == WM_APP) { RunJobs(); continue; }
                TranslateMessage(ref msg);
                DispatchMessage(ref msg);
            }
            RunJobs();
        }

        void RunJobs()
        {
            for (;;)
            {
                Job job;
                lock (jobs) { if (jobs.Count == 0) return; job = jobs.Dequeue(); }
                try { job.Result = job.Work(); }
                catch (Exception e) { job.Error = e; }
                job.Done.Set();
            }
        }

        /// Runs `work` on the UI thread and returns its result.
        string Invoke(Func<string> work)
        {
            Job job = new Job();
            job.Work = work;
            lock (jobs) jobs.Enqueue(job);
            // A thread message can be lost in a modal loop (a window being
            // dragged); post again until the job has run.
            for (int i = 0; i < 100; i++)
            {
                PostThreadMessage(uiThreadId, WM_APP, IntPtr.Zero, IntPtr.Zero);
                if (job.Done.WaitOne(100)) break;
            }
            if (!job.Done.WaitOne(0)) throw new TimeoutException("the window thread did not answer");
            if (job.Error != null) throw job.Error;
            return job.Result;
        }

        Table Find(string id)
        {
            foreach (Table t in tables) if (t.Id == id) return t;
            return null;
        }

        Table FindHwnd(IntPtr hwnd)
        {
            foreach (Table t in tables) if (t.Hwnd == hwnd) return t;
            return null;
        }

        // ------------------------------------------------------------ commands

        public string Open(string id, string title, int x, int y, int width, int height, int seats)
        {
            return Invoke(delegate ()
            {
                if (Find(id) != null) throw new ArgumentException("table " + id + " is already open");
                IntPtr hwnd = CreateWindowEx(0, ClassName, title, WS_OVERLAPPEDWINDOW, x, y, width, height, IntPtr.Zero, IntPtr.Zero, GetModuleHandle(null), IntPtr.Zero);
                if (hwnd == IntPtr.Zero) throw new InvalidOperationException("CreateWindowEx failed: " + Marshal.GetLastWin32Error());
                Table t = new Table();
                t.Id = id; t.Hwnd = hwnd; t.Title = title; t.Seats = seats;
                tables.Add(t);
                ShowWindow(hwnd, 4); // SW_SHOWNOACTIVATE
                UpdateWindow(hwnd);
                AssertTitle(t);
                WriteStatus();
                return TableJson(t);
            });
        }

        static void AssertTitle(Table t)
        {
            string seen = ReadTitle(t.Hwnd);
            if (seen != t.Title) throw new InvalidOperationException("table " + t.Id + ": Windows reports the title as \"" + seen + "\", not \"" + t.Title + "\"");
        }

        public string Move(string id, int x, int y, int width, int height)
        {
            return Invoke(delegate ()
            {
                Table t = Require(id);
                SetWindowPos(t.Hwnd, IntPtr.Zero, x, y, width, height, SWP_NOZORDER | SWP_NOACTIVATE);
                InvalidateRect(t.Hwnd, IntPtr.Zero, true);
                WriteStatus();
                return TableJson(t);
            });
        }

        public string Retitle(string id, string title)
        {
            return Invoke(delegate ()
            {
                Table t = Require(id);
                SetWindowText(t.Hwnd, title);
                t.Title = title;
                AssertTitle(t);
                WriteStatus();
                return TableJson(t);
            });
        }

        public string Close(string id)
        {
            return Invoke(delegate ()
            {
                Table t = Require(id);
                DestroyWindow(t.Hwnd);
                tables.Remove(t);
                WriteStatus();
                return "{\"closed\":\"" + Esc(id) + "\"}";
            });
        }

        public string CloseAll()
        {
            return Invoke(delegate ()
            {
                int n = tables.Count;
                foreach (Table t in tables.ToArray()) DestroyWindow(t.Hwnd);
                tables.Clear();
                WriteStatus();
                return "{\"closed\":" + n + "}";
            });
        }

        public string Shot(string path, int x, int y, int width, int height)
        {
            return Invoke(delegate ()
            {
                if (width <= 0 || height <= 0) throw new ArgumentException("empty capture region");
                using (Bitmap bmp = new Bitmap(width, height, PixelFormat.Format32bppArgb))
                {
                    using (Graphics g = Graphics.FromImage(bmp))
                    {
                        // CAPTUREBLT includes layered windows: the transparent overlay.
                        // BitBlt directly: CopyFromScreen rejects SourceCopy | CaptureBlt
                        // as an undefined CopyPixelOperation value.
                        IntPtr screen = GetDC(IntPtr.Zero);
                        IntPtr dest = g.GetHdc();
                        try
                        {
                            if (!BitBlt(dest, 0, 0, width, height, screen, x, y, SRCCOPY | CAPTUREBLT))
                                throw new InvalidOperationException("BitBlt failed: " + Marshal.GetLastWin32Error());
                        }
                        finally
                        {
                            g.ReleaseHdc(dest);
                            ReleaseDC(IntPtr.Zero, screen);
                        }
                    }
                    bmp.Save(path, ImageFormat.Png);
                }
                return "{\"path\":\"" + Esc(path) + "\",\"width\":" + width + ",\"height\":" + height + "}";
            });
        }

        // Renders one fake table's own window to a PNG (PrintWindow: no
        // screen involved, so it also works on a locked session or under
        // another window), then draws `overlayPath` (a PNG the size of the
        // window, transparent outside the HUD) on top if given.
        public string Print(string id, string path, string overlayPath)
        {
            return Invoke(delegate ()
            {
                Table t = Require(id);
                RECT r;
                if (!GetWindowRect(t.Hwnd, out r)) throw new InvalidOperationException("GetWindowRect failed");
                int width = r.Right - r.Left, height = r.Bottom - r.Top;
                if (width <= 0 || height <= 0) throw new ArgumentException("empty window");
                using (Bitmap bmp = new Bitmap(width, height, PixelFormat.Format32bppArgb))
                {
                    using (Graphics g = Graphics.FromImage(bmp))
                    {
                        IntPtr dest = g.GetHdc();
                        bool printed;
                        try { printed = PrintWindow(t.Hwnd, dest, PW_RENDERFULLCONTENT); }
                        finally { g.ReleaseHdc(dest); }
                        if (!printed) throw new InvalidOperationException("PrintWindow failed");
                        if (!string.IsNullOrEmpty(overlayPath))
                        {
                            using (Image hud = Image.FromFile(overlayPath))
                            {
                                g.DrawImage(hud, new Rectangle(0, 0, width, height));
                            }
                        }
                    }
                    bmp.Save(path, ImageFormat.Png);
                }
                return "{\"path\":\"" + Esc(path) + "\",\"width\":" + width + ",\"height\":" + height + "}";
            });
        }

        public string Mouse(int x, int y, bool click)
        {
            return Invoke(delegate ()
            {
                SetCursorPos(x, y);
                if (click)
                {
                    mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, UIntPtr.Zero);
                    mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, UIntPtr.Zero);
                }
                POINT p;
                GetCursorPos(out p);
                return "{\"x\":" + p.X + ",\"y\":" + p.Y + "}";
            });
        }

        /// Presses a key combination (virtual-key code `vk` with modifiers), e.g. Ctrl+Alt+P.
        public string Keys(bool ctrl, bool alt, bool shift, int vk)
        {
            return Invoke(delegate ()
            {
                const uint KEYUP = 0x0002;
                List<byte> mods = new List<byte>();
                if (ctrl) mods.Add(0x11);
                if (alt) mods.Add(0x12);
                if (shift) mods.Add(0x10);
                foreach (byte m in mods) keybd_event(m, 0, 0, UIntPtr.Zero);
                keybd_event((byte)vk, 0, 0, UIntPtr.Zero);
                keybd_event((byte)vk, 0, KEYUP, UIntPtr.Zero);
                for (int i = mods.Count - 1; i >= 0; i--) keybd_event(mods[i], 0, KEYUP, UIntPtr.Zero);
                return "{\"vk\":" + vk + "}";
            });
        }

        /// ShowWindow on any top-level window (6 minimizes, 9 restores).
        public string Show(long hwnd, int cmd)
        {
            return Invoke(delegate ()
            {
                IntPtr h = new IntPtr(hwnd);
                if (!IsWindow(h)) throw new ArgumentException("no window " + hwnd);
                ShowWindow(h, cmd);
                return "{\"hwnd\":" + hwnd + ",\"cmd\":" + cmd + "}";
            });
        }

        /// Top-level windows (visible or not) whose title contains `needle`, any process.
        public string List(string needle)
        {
            return Invoke(delegate ()
            {
                StringBuilder sb = new StringBuilder("[");
                bool first = true;
                EnumWindows(delegate (IntPtr h, IntPtr d)
                {
                    StringBuilder title = new StringBuilder(512);
                    GetWindowText(h, title, title.Capacity);
                    string text = title.ToString();
                    if (needle != null && text.IndexOf(needle, StringComparison.Ordinal) < 0) return true;
                    StringBuilder cls = new StringBuilder(256);
                    GetClassName(h, cls, cls.Capacity);
                    RECT r;
                    GetWindowRect(h, out r);
                    uint pid;
                    GetWindowThreadProcessId(h, out pid);
                    if (!first) sb.Append(",");
                    first = false;
                    sb.Append("{\"hwnd\":").Append(h.ToInt64()).Append(",\"pid\":").Append(pid);
                    sb.Append(",\"title\":\"").Append(Esc(text)).Append("\",\"className\":\"").Append(Esc(cls.ToString()));
                    sb.Append("\",\"visible\":").Append(IsWindowVisible(h) ? "true" : "false");
                    sb.Append(",\"rect\":").Append(RectJson(r)).Append("}");
                    return true;
                }, IntPtr.Zero);
                return sb.Append("]").ToString();
            });
        }

        public string Status() { return Invoke(delegate () { return StatusJson(); }); }

        public void Stop()
        {
            try { CloseAll(); } catch (Exception) { }
            PostThreadMessage(uiThreadId, 0x0012 /* WM_QUIT */, IntPtr.Zero, IntPtr.Zero);
            if (ui != null) ui.Join(3000);
        }

        Table Require(string id)
        {
            Table t = Find(id);
            if (t == null) throw new ArgumentException("no open table " + id);
            return t;
        }

        // ------------------------------------------------------------ window procedure

        IntPtr WindowProc(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam)
        {
            // An exception must never unwind through the native message loop.
            try { return HandleMessage(hWnd, msg, wParam, lParam); }
            catch (Exception) { return DefWindowProc(hWnd, msg, wParam, lParam); }
        }

        IntPtr HandleMessage(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam)
        {
            Table t = FindHwnd(hWnd);
            switch (msg)
            {
                case WM_ERASEBKGND:
                    return new IntPtr(1);
                case WM_SIZE:
                    InvalidateRect(hWnd, IntPtr.Zero, false);
                    break;
                case WM_PAINT:
                    {
                        PAINTSTRUCT ps;
                        IntPtr hdc = BeginPaint(hWnd, out ps);
                        try { if (t != null) Paint(t, hdc); }
                        finally { EndPaint(hWnd, ref ps); }
                        return IntPtr.Zero;
                    }
                case WM_LBUTTONDOWN:
                    if (t != null)
                    {
                        POINT p = new POINT();
                        p.X = (short)(lParam.ToInt64() & 0xFFFF);
                        p.Y = (short)((lParam.ToInt64() >> 16) & 0xFFFF);
                        ClientToScreen(hWnd, ref p);
                        t.Clicks++;
                        t.LastClickX = p.X; t.LastClickY = p.Y; t.HasClick = true;
                        InvalidateRect(hWnd, IntPtr.Zero, false);
                        WriteStatus();
                    }
                    return IntPtr.Zero;
                case WM_CLOSE:
                    if (t != null) { tables.Remove(t); WriteStatus(); }
                    DestroyWindow(hWnd);
                    return IntPtr.Zero;
            }
            return DefWindowProc(hWnd, msg, wParam, lParam);
        }

        // Seat plates in the hero frame (hero bottom centre, clockwise), as
        // fractions of the whole window: src/overlay/seatLayout.ts.
        static readonly float[][] Plates6 = {
            new float[] {0.5f,0.79f}, new float[] {0.12f,0.58f}, new float[] {0.12f,0.28f},
            new float[] {0.5f,0.19f}, new float[] {0.88f,0.28f}, new float[] {0.88f,0.58f} };
        static readonly float[][] Plates9 = {
            new float[] {0.5f,0.79f}, new float[] {0.25f,0.75f}, new float[] {0.09f,0.57f}, new float[] {0.09f,0.33f},
            new float[] {0.3f,0.18f}, new float[] {0.7f,0.18f}, new float[] {0.91f,0.33f}, new float[] {0.91f,0.57f},
            new float[] {0.75f,0.75f} };

        static float[][] PlatesFor(int seats)
        {
            if (seats == 6) return Plates6;
            if (seats == 9) return Plates9;
            int n = Math.Max(2, Math.Min(10, seats));
            float[][] ring = new float[n][];
            for (int i = 0; i < n; i++)
            {
                double theta = (90.0 + i * 360.0 / n) * Math.PI / 180.0;
                ring[i] = new float[] {
                    (float)Math.Min(0.91, Math.Max(0.09, 0.5 + 0.4 * Math.Cos(theta))),
                    (float)Math.Min(0.79, Math.Max(0.18, 0.47 + 0.3 * Math.Sin(theta))) };
            }
            return ring;
        }

        void Paint(Table t, IntPtr hdc)
        {
            RECT wr, cr;
            GetWindowRect(t.Hwnd, out wr);
            GetClientRect(t.Hwnd, out cr);
            POINT origin = new POINT();
            ClientToScreen(t.Hwnd, ref origin);
            float ww = wr.Right - wr.Left, wh = wr.Bottom - wr.Top;
            float ox = origin.X - wr.Left, oy = origin.Y - wr.Top;
            // Window fraction -> client pixel.
            Func<float, float> X = delegate (float f) { return f * ww - ox; };
            Func<float, float> Y = delegate (float f) { return f * wh - oy; };
            int cw = Math.Max(1, cr.Right), ch = Math.Max(1, cr.Bottom);
            using (Bitmap buffer = new Bitmap(cw, ch))
            using (Graphics g = Graphics.FromImage(buffer))
            using (Font font = new Font("Segoe UI", Math.Max(7f, wh / 60f), FontStyle.Bold, GraphicsUnit.Pixel))
            using (StringFormat centre = new StringFormat())
            {
                centre.Alignment = StringAlignment.Center;
                centre.LineAlignment = StringAlignment.Center;
                g.SmoothingMode = SmoothingMode.AntiAlias;
                g.Clear(Color.FromArgb(18, 20, 26));
                using (SolidBrush rail = new SolidBrush(Color.FromArgb(60, 38, 24)))
                    g.FillEllipse(rail, X(0.06f), Y(0.12f), X(0.94f) - X(0.06f), Y(0.86f) - Y(0.12f));
                using (SolidBrush felt = new SolidBrush(Color.FromArgb(22, 104, 64)))
                    g.FillEllipse(felt, X(0.09f), Y(0.15f), X(0.91f) - X(0.09f), Y(0.83f) - Y(0.15f));
                // Board: five card slots.
                using (Pen outline = new Pen(Color.FromArgb(140, 255, 255, 255), 1f))
                {
                    float bx = X(0.33f), by = Y(0.40f), bw = X(0.67f) - X(0.33f), bh = Y(0.50f) - Y(0.40f);
                    for (int i = 0; i < 5; i++) g.DrawRectangle(outline, bx + i * bw / 5f + 1, by, bw / 5f - 3, bh);
                }
                float[][] plates = PlatesFor(t.Seats);
                using (SolidBrush plate = new SolidBrush(Color.FromArgb(30, 30, 36)))
                using (SolidBrush heroPlate = new SolidBrush(Color.FromArgb(40, 52, 90)))
                using (SolidBrush cardBack = new SolidBrush(Color.FromArgb(150, 28, 36)))
                using (SolidBrush text = new SolidBrush(Color.FromArgb(230, 230, 230)))
                using (Pen edge = new Pen(Color.FromArgb(200, 200, 200), 1f))
                {
                    for (int i = 0; i < plates.Length; i++)
                    {
                        float px = plates[i][0], py = plates[i][1];
                        // Hole cards above the plate (HOLE_CARDS in seatLayout.ts).
                        float top = py - 0.04f + 0.02f - 0.13f;
                        g.FillRectangle(cardBack, X(px - 0.06f), Y(top), X(px) - X(px - 0.06f) - 1, Y(top + 0.13f) - Y(top));
                        g.FillRectangle(cardBack, X(px) + 1, Y(top), X(px + 0.06f) - X(px) - 1, Y(top + 0.13f) - Y(top));
                        RectangleF r = new RectangleF(X(px - 0.075f), Y(py - 0.04f), X(px + 0.075f) - X(px - 0.075f), Y(py + 0.04f) - Y(py - 0.04f));
                        g.FillRectangle(i == 0 ? heroPlate : plate, r);
                        g.DrawRectangle(edge, r.X, r.Y, r.Width, r.Height);
                        g.DrawString(i == 0 ? "Hero" : "Seat " + (i + 1), font, text, r, centre);
                    }
                    // Action buttons (ACTIONS_ZONE in seatLayout.ts).
                    string[] labels = { "Fold", "Call", "Raise" };
                    using (SolidBrush button = new SolidBrush(Color.FromArgb(120, 20, 20)))
                    {
                        float ax = X(0.58f), ay = Y(0.88f), aw = (X(0.99f) - X(0.58f)) / 3f, ah = Y(0.97f) - Y(0.88f);
                        for (int i = 0; i < 3; i++)
                        {
                            RectangleF b = new RectangleF(ax + i * aw + 2, ay, aw - 4, ah);
                            g.FillRectangle(button, b);
                            g.DrawString(labels[i], font, text, b, centre);
                        }
                    }
                    g.DrawString(t.Id + "  clicks: " + t.Clicks, font, text, 4f, 4f);
                }
                using (Graphics screen = Graphics.FromHdc(hdc)) screen.DrawImageUnscaled(buffer, 0, 0);
            }
        }

        // ------------------------------------------------------------ status

        void WriteStatus()
        {
            if (string.IsNullOrEmpty(statusPath)) return;
            string json = StatusJson();
            string tmp = statusPath + ".tmp";
            // A reader holding the file open makes the swap fail; try again
            // briefly, and never let it take the window thread down.
            for (int attempt = 0; attempt < 20; attempt++)
            {
                try
                {
                    File.WriteAllText(tmp, json, new UTF8Encoding(false));
                    if (File.Exists(statusPath)) File.Replace(tmp, statusPath, null);
                    else File.Move(tmp, statusPath);
                    return;
                }
                catch (IOException) { Thread.Sleep(10); }
                catch (UnauthorizedAccessException) { Thread.Sleep(10); }
            }
        }

        string StatusJson()
        {
            StringBuilder sb = new StringBuilder("{\"className\":\"" + ClassName + "\",\"updated\":\"");
            sb.Append(DateTime.Now.ToString("o")).Append("\",\"tables\":[");
            for (int i = 0; i < tables.Count; i++)
            {
                if (i > 0) sb.Append(",");
                sb.Append(TableJson(tables[i]));
            }
            return sb.Append("]}").ToString();
        }

        string TableJson(Table t)
        {
            RECT r;
            GetWindowRect(t.Hwnd, out r);
            bool alive = IsWindow(t.Hwnd);
            StringBuilder sb = new StringBuilder();
            sb.Append("{\"id\":\"").Append(Esc(t.Id)).Append("\",\"hwnd\":").Append(t.Hwnd.ToInt64());
            sb.Append(",\"title\":\"").Append(Esc(t.Title)).Append("\",\"seats\":").Append(t.Seats);
            sb.Append(",\"alive\":").Append(alive ? "true" : "false");
            sb.Append(",\"rect\":").Append(RectJson(r)).Append(",\"clicks\":").Append(t.Clicks);
            if (t.HasClick) sb.Append(",\"lastClick\":{\"x\":").Append(t.LastClickX).Append(",\"y\":").Append(t.LastClickY).Append("}");
            return sb.Append("}").ToString();
        }

        static string RectJson(RECT r)
        {
            return "{\"x\":" + r.Left + ",\"y\":" + r.Top + ",\"width\":" + (r.Right - r.Left) + ",\"height\":" + (r.Bottom - r.Top) + "}";
        }

        static string Esc(string s)
        {
            if (s == null) return "";
            StringBuilder sb = new StringBuilder();
            foreach (char c in s)
            {
                if (c == '"' || c == '\\') sb.Append('\\').Append(c);
                else if (c < 0x20) sb.Append("\\u").Append(((int)c).ToString("x4"));
                else sb.Append(c);
            }
            return sb.ToString();
        }
    }
}
'@

Add-Type -TypeDefinition $source -ReferencedAssemblies System.Drawing

if ($Probe) {
  [Console]::Out.WriteLine([VeloraSim.FakeTables]::Probe())
  exit 0
}
if (-not $Status) {
  [Console]::Error.WriteLine('fake-tables: -Status <file> or -Probe is required')
  exit 2
}

$app = New-Object VeloraSim.FakeTables($Status)
$app.Start()
[Console]::Out.WriteLine('{"ready":true}')
[Console]::Out.Flush()

function Reply($seq, $body) {
  if ($null -eq $seq) { $seq = 'null' }
  [Console]::Out.WriteLine('{"seq":' + $seq + ',' + $body + '}')
  [Console]::Out.Flush()
}

try {
  while ($null -ne ($line = [Console]::In.ReadLine())) {
    if ($line.Trim() -eq '') { continue }
    $seq = $null
    try {
      $cmd = $line | ConvertFrom-Json
      $seq = $cmd.seq
      $result = switch ($cmd.op) {
        'open'    { $app.Open([string]$cmd.id, [string]$cmd.title, [int]$cmd.x, [int]$cmd.y, [int]$cmd.width, [int]$cmd.height, [int]$cmd.seats) }
        'move'    { $app.Move([string]$cmd.id, [int]$cmd.x, [int]$cmd.y, [int]$cmd.width, [int]$cmd.height) }
        'retitle' { $app.Retitle([string]$cmd.id, [string]$cmd.title) }
        'close'   { $app.Close([string]$cmd.id) }
        'closeAll' { $app.CloseAll() }
        'shot'    { $app.Shot([string]$cmd.path, [int]$cmd.x, [int]$cmd.y, [int]$cmd.width, [int]$cmd.height) }
        'print'   { $app.Print([string]$cmd.id, [string]$cmd.path, [string]$cmd.overlay) }
        'mouse'   { $app.Mouse([int]$cmd.x, [int]$cmd.y, $false) }
        'click'   { $app.Mouse([int]$cmd.x, [int]$cmd.y, $true) }
        'keys'    { $app.Keys([bool]$cmd.ctrl, [bool]$cmd.alt, [bool]$cmd.shift, [int]$cmd.vk) }
        'show'    { $app.Show([long]$cmd.hwnd, [int]$cmd.cmd) }
        'list'    { $app.List([string]$cmd.title) }
        'status'  { $app.Status() }
        'quit'    { '{"quit":true}' }
        default   { throw "unknown op $($cmd.op)" }
      }
      Reply $seq ('"ok":true,"result":' + $result)
      if ($cmd.op -eq 'quit') { break }
    } catch {
      $message = ($_.Exception.Message -replace '\\', '\\' -replace '"', '\"' -replace "[`r`n]", ' ')
      Reply $seq ('"ok":false,"error":"' + $message + '"')
    }
  }
} finally {
  $app.Stop()
}
