#!/usr/bin/env python3
"""Render an intro film: the first-run one, or the one for a step.

The first-run film, resources/intro/intro.mp4, is drawn by scripts/intro/film.html;
a step's, such as resources/intro/organize.mp4, by its own page, scripts/intro/organize.html.
This script steps through one a frame at a time in headless Chromium and encodes
the frames with ffmpeg, then adds the sound that scripts/intro/sound.py synthesizes.

Needs `pip install playwright numpy scipy`, ffmpeg, and either `playwright install
chromium` or Google Chrome.

    python3 scripts/render_intro.py                 # the whole first-run film
    python3 scripts/render_intro.py --film organize # the Organize step's film
    python3 scripts/render_intro.py --sound-only    # keep the picture, redo the sound
    python3 scripts/render_intro.py --stills 1 5 9  # PNG frames at those seconds
"""

import argparse
import functools
import http.server
import subprocess
import sys
import tempfile
import threading
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parent.parent
FPS = 30
sys.path.insert(0, str(ROOT / "scripts/intro"))
import sound  # noqa: E402


def add_sound(picture, out, film):
    """Writes `out` as `picture`'s video, untouched, with `film`'s sound."""
    with tempfile.TemporaryDirectory() as tmp:
        track, mixed = Path(tmp) / "sound.wav", Path(tmp) / "intro.mp4"
        sound.write(track, film)
        subprocess.run(
            ["ffmpeg", "-nostdin", "-v", "error", "-y", "-i", str(picture), "-i", str(track),
             "-map", "0:v", "-map", "1:a", "-c:v", "copy", "-c:a", "aac", "-b:a", "160k",
             "-movflags", "+faststart", str(mixed)],
            check=True,
        )
        mixed.replace(out)


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--film", choices=sorted(sound.FILMS), default="intro")
    parser.add_argument("--out", type=Path, help="default: resources/intro/FILM.mp4")
    parser.add_argument("--stills", type=float, nargs="+", metavar="SECONDS")
    parser.add_argument("--stills-dir", type=Path, default=Path("."))
    parser.add_argument("--sound-only", action="store_true")
    args = parser.parse_args()
    args.out = args.out or ROOT / f"resources/intro/{args.film}.mp4"
    page_name = "film" if args.film == "intro" else args.film
    if args.sound_only:
        add_sound(args.out, args.out, args.film)
        return

    server = http.server.ThreadingHTTPServer(
        ("127.0.0.1", 0), functools.partial(Quiet, directory=str(ROOT))
    )
    threading.Thread(target=server.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{server.server_port}/scripts/intro/{page_name}.html?render"

    with sync_playwright() as p:
        try:
            browser = p.chromium.launch()
        except Exception:
            # No Playwright Chromium on this machine: use the installed Chrome.
            browser = p.chromium.launch(channel="chrome")
        page = browser.new_page(viewport={"width": 1920, "height": 1080}, device_scale_factor=1)
        page.goto(url)
        page.evaluate("window.ready")
        duration = page.evaluate("window.DURATION")

        def frame(seconds):
            page.evaluate("t => window.draw(t)", seconds)
            # A screenshot can time out while the machine is busy or its
            # display is asleep; the frame depends on nothing else, so retry.
            for attempt in range(5):
                try:
                    return page.screenshot(type="png", timeout=120_000)
                except Exception:
                    if attempt == 4:
                        raise

        if args.stills:
            args.stills_dir.mkdir(parents=True, exist_ok=True)
            for seconds in args.stills:
                (args.stills_dir / f"{args.film}-{seconds:05.2f}.png").write_bytes(frame(seconds))
        else:
            frames = round(duration * FPS)
            # Rendered beside the film and moved over it only when complete,
            # so a render that stops never leaves half a film behind.
            picture = args.out.with_name(args.out.stem + ".rendering.mp4")
            encoder = subprocess.Popen(
                ["ffmpeg", "-v", "error", "-y", "-f", "image2pipe", "-framerate", str(FPS),
                 "-c:v", "png", "-i", "-",
                 "-vf", "scale=out_color_matrix=bt709:out_range=tv",
                 "-c:v", "libx264", "-profile:v", "high", "-preset", "slow", "-crf", "27",
                 "-pix_fmt", "yuv420p", "-colorspace", "bt709", "-color_primaries", "bt709",
                 "-color_trc", "bt709", "-color_range", "tv", "-movflags", "+faststart", "-an",
                 str(picture)],
                stdin=subprocess.PIPE,
            )
            for n in range(frames):
                encoder.stdin.write(frame(n / FPS))
            encoder.stdin.close()
            if encoder.wait() != 0:
                raise SystemExit("ffmpeg failed")
            add_sound(picture, picture, args.film)
            picture.replace(args.out)
            print(f"{args.out}: {frames} frames, {frames / FPS:.1f} s")
        browser.close()
    server.shutdown()


if __name__ == "__main__":
    main()
