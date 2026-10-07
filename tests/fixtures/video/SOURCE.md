# Synthetic video fixtures

Generated locally with FFmpeg 8 from test sources; no photographs or third-party
media are included. `motion.mp4` is three seconds of 320×180 H.264 motion plus
440 Hz AAC audio. `rotated.mp4` remuxes it with a 90-degree display transform.
`large.mp4` is half a second of blue 3840×2160 H.264, used to verify bounded
output geometry and channel order. Encoding bytes can vary with FFmpeg versions.

Equivalent generation commands:

```sh
ffmpeg -f lavfi -i testsrc2=size=320x180:rate=30 -f lavfi -i sine=frequency=440:sample_rate=48000 -t 3 -c:v libx264 -pix_fmt yuv420p -c:a aac -movflags +faststart motion.mp4
ffmpeg -display_rotation 90 -i motion.mp4 -c copy rotated.mp4
ffmpeg -f lavfi -i color=c=blue:size=3840x2160:rate=30 -t 0.5 -c:v libx264 -pix_fmt yuv420p large.mp4
```

Run `cargo run --locked --example video_probe` on macOS for the windowless,
muted native decoder check. It uses temporary cache storage and verifies input
content hashes before returning. Normal tests use a fake playback backend to
exercise cancellation and command races without depending on a system codec.
