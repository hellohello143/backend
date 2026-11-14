// The region request setup follows CaptureRegion.cs from ShareX, copyright 2007–2026 ShareX Team.
// Distributed under GPL-3.0-or-later. The overlay, editor, and screenshot assemblies are upstream ShareX.
using System.Drawing;
using System.Globalization;
using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using ShareX.AvaloniaUI.Integration;
using ShareX.HelpersLib;
using ShareX.ImageEditor.Integration;
using ShareX.ScreenCaptureLib;
using ShareX.ScreenCaptureLib.Presentation.RegionCapture;
using SkiaSharp;

public static class StartupHook
{
    // .NET invokes this entry point before ShareX.Main. The original executable
    // supplies its app host, dependency manifest, and self-contained runtime.
    [SupportedOSPlatform("windows10.0.22621")]
    public static void Initialize()
    {
        string? output = Environment.GetEnvironmentVariable("KUMA_SHAREX_OUTPUT");
        if (string.IsNullOrEmpty(output))
            return;

        int exit = 1;
        var thread = new Thread(() => exit = Run(output));
        thread.SetApartmentState(ApartmentState.STA);
        thread.Start();
        thread.Join();
        Environment.Exit(exit);
    }

    [SupportedOSPlatform("windows10.0.22621")]
    private static int Run(string output)
    {
        SetProcessDpiAwarenessContext(-4);
        string mode = Environment.GetEnvironmentVariable("KUMA_SHAREX_MODE") ?? "region";
        bool cursor = Environment.GetEnvironmentVariable("KUMA_SHAREX_CURSOR") == "1";
        try
        {
            if (mode == "probe")
            {
                Version revision = typeof(Screenshot).Assembly.GetName().Version
                    ?? throw new InvalidOperationException("ShareX assembly version is unavailable.");
                File.WriteAllText(output, revision.ToString(3));
                return 0;
            }
            if (mode != "region")
            {
                using SKBitmap? bitmap = ScreenshotForMode(mode, cursor);
                if (bitmap is null)
                    throw new InvalidOperationException("ShareX returned no screenshot.");
                WritePng(bitmap, output);
                return 0;
            }

            int exit = 1;
            AvaloniaBootstrapper.Initialize([], async () =>
            {
                try
                {
                    await CaptureRegion(output, cursor);
                    exit = 0;
                }
                catch (Exception exception)
                {
                    WriteError(output, exception);
                }
                finally
                {
                    AvaloniaBootstrapper.Shutdown();
                }
            }, () => { });
            AvaloniaBootstrapper.Run();
            return exit;
        }
        catch (Exception exception)
        {
            WriteError(output, exception);
            return 1;
        }
    }

    [SupportedOSPlatform("windows10.0.22621")]
    private static SKBitmap? ScreenshotForMode(string mode, bool cursor)
    {
        var screenshot = new Screenshot { CaptureCursor = cursor };
        string bounds = Environment.GetEnvironmentVariable("KUMA_SHAREX_BOUNDS") ?? "";
        string[] parts = bounds.Split(',');
        return mode switch
        {
            "desktop" => screenshot.CaptureFullscreen(),
            "monitor" when parts.Length == 4 => screenshot.CaptureRectangle(new Rectangle(
                Parse(parts[0]), Parse(parts[1]), Parse(parts[2]), Parse(parts[3]))),
            "window" => screenshot.CaptureWindow(new IntPtr(long.Parse(bounds, CultureInfo.InvariantCulture))),
            _ => throw new ArgumentException("Unsupported screenshot mode.")
        };
    }

    private static int Parse(string value) => int.Parse(value, CultureInfo.InvariantCulture);

    [SupportedOSPlatform("windows10.0.22621")]
    private static async Task CaptureRegion(string output, bool includeCursor)
    {
        var screenshot = new Screenshot { CaptureCursor = false };
        Rectangle bounds = CaptureHelpers.GetScreenBounds();
        SKBitmap frozen;
        using (SKBitmap canvas = screenshot.CaptureFullscreen())
        {
            frozen = canvas.Copy();
        }

        SKBitmap? cursor = null;
        Point position = Point.Empty;
        if (includeCursor)
        {
            var data = new CursorData();
            if (data.IsVisible)
            {
                using SKBitmap bitmap = data.ToBitmap(frozen, bounds.Location);
                cursor = bitmap.Copy();
                position = new Point(data.DrawPosition.X - bounds.X, data.DrawPosition.Y - bounds.Y);
            }
        }

        var result = await RegionCaptureIntegration.CaptureAsync(new AvaloniaRegionCaptureRequest
        {
            Screenshot = frozen,
            ScreenBounds = bounds,
            RegionCaptureOptions = new RegionCaptureOptions(),
            ImageEditorOptions = new ImageEditorOptions(),
            EnableAnnotations = true,
            AllowMultipleRegions = true,
            CursorBitmap = cursor,
            CursorPosition = position
        });
        if (result is not null)
        {
            using SKBitmap image = result.Image;
            WritePng(image, output);
        }
    }

    [SupportedOSPlatform("windows10.0.22621")]
    private static void WritePng(SKBitmap bitmap, string output)
    {
        using SKImage image = SKImage.FromBitmap(bitmap);
        using SKData encoded = image.Encode(SKEncodedImageFormat.Png, 100)
            ?? throw new InvalidOperationException("Could not encode the ShareX screenshot.");
        using FileStream file = File.Create(output);
        encoded.SaveTo(file);
    }

    private static void WriteError(string output, Exception error)
    {
        try { File.WriteAllText(Path.ChangeExtension(output, ".error"), error.Message); }
        catch (IOException) { }
        catch (UnauthorizedAccessException) { }
    }

    [DllImport("user32.dll")]
    private static extern bool SetProcessDpiAwarenessContext(nint value);
}
