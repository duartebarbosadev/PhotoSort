# Native inference backends

Settings → AI execution selects CPU, Automatic, Apple Core ML, NVIDIA CUDA or
Windows DirectML. CPU is the default, including for existing preferences. Models
are loaded lazily by the existing image worker; opening the application or Settings
does not initialize inference. Changes apply to the next accepted analysis or
rotation run. The same registry serves all five workflows, including lazy face
comparisons and regional crops. Image preparation, metadata and evidence caches
remain shared and reusable after a backend change.

Automatic tries Core ML on macOS, CUDA then DirectML on Windows, and CUDA on
Linux. A provider must be included in the installed native ONNX Runtime and have
its required hardware/runtime dependencies. Explicit unsupported selections also
fall back to CPU and explain why. CUDA/DirectML dispatch is implemented but has
not been executed on their hardware in this macOS validation. Other GPU/NPU
providers, AMD ROCm on Linux, multi-GPU selection and packaged Windows/Linux
GPU runtimes are outside this implementation. The CI matrix now compile-checks
ONNX-enabled code on all three operating systems; CI has not been run here.

The application tries registration and model creation explicitly, rather than
silently accepting an unavailable provider. A native execution error releases
that model's accelerated session, retries the same prepared tensor once on CPU,
and keeps the CPU session for subsequent calls. No new image decoding is required.
Changing the selected backend or model file allows another initialization attempt.
A cancelled or obsolete run cannot start another provider or CPU fallback.
Native compilation and an individual synchronous native call cannot be interrupted;
browsing and cancellation remain on the UI thread and stale results are discarded.

The footer shows model preparation while a run is active. Settings lists the
provider configured for each session and CPU fallback reasons. A configured
provider does not mean all graph operations ran on GPU/Neural Engine: unsupported
operators can remain on CPU. Actual placement requires native profiling.

## Apple

The bundled macOS runtime includes Core ML. The DINO export's four symbolic axes
are fixed at 1×3×224×224 for Core ML, allowing the measured graph to form one
partition. Regional crops run sequentially through this same fixed session with
cancellation between calls, avoiding multiple compiled DINO instances. CPU and
CUDA retain the export's dynamic batch support.

An isolated release probe on this Mac measured a warm median of approximately
29.0 ms on CPU and 3.73 ms with fixed-shape Core ML (about 7.8× for that model call).
Without shape specialization, Core ML was slower (69.5 ms). This is synthetic
model inference, not end-to-end grouping, a broad-library benchmark or a Python
comparison. See `diagnostics/acceleration-probe-2026-10-04.json`. The adjacent
standalone Rust source preserves that measurement harness; its direct session
construction is intentionally isolated validation code, not an application owner.

Cold compilation matters: the isolated DINO probe took about 29.6 seconds to
construct its session, plus about 1.6 seconds for its first call, and peaked at
about 1.94 GB process RSS. In a separate debug runtime check, aesthetic-model
preparation and first inference took about 114 seconds. Five model sessions
accepted Core ML; the face-landmark export failed Core ML compilation and the
shared service continued on CPU. All six models then passed a second inference
pass without reinitialization. These are interface/reuse checks, not broad
photographic accuracy measurements.

The shared model service now enables ONNX Runtime's persistent Core ML cache for
verified, self-contained catalog models when the disk-cache budget is nonzero.
Stable directories are keyed by model/runtime contents, platform and provider
recipe; checksummed manifests reject incomplete or changed native files. OS-held
session locks prevent eviction while a model uses its files. These directories
share the photo-cache disk budget. Clear or a budget reduction releases this
window's prepared sessions on the image worker; files held by another session
are retained and reported. Active model files may temporarily exceed the limit;
oversize entries are not retained after their session closes. Model assets and
reviews are not removed. Custom exports remain usable without persistent caching.

First compilation and runtime/OS temporary caches remain outside that persistence
budget. Native compilation itself still cannot be interrupted by the current ORT
call. CPU remains the low-initialization-cost default; acceleration is opt-in.
Model and native-runtime memory is additional to the preview-cache allowance.
Source integration has passed the seven-sample native lifecycle check, all three
full suites and strict Clippy. The macOS native-imaging bundle has been rebuilt and its signature verified.
Native Settings, cancellation/retry, restart reuse and cache clearing now pass on
nine disposable photos. DINOv2 and face detection reused identical prepared native
files after restart; face landmarks reported its compiler error and ran on CPU.
An initial Recent-folder discovery stall remains unresolved after successful
retries. See `diagnostics/coreml-desktop-2026-10-06/README.md`; the earlier automated
lifecycle evidence is in `diagnostics/coreml-cache-integration-2026-10-06/README.md`.

Core ML configuration follows the [official provider documentation](https://onnxruntime.ai/docs/execution-providers/CoreML-ExecutionProvider.html).

## NVIDIA and other Windows GPUs

CUDA uses heuristic convolution selection, limited convolution workspace,
arena growth based on requested allocations and FP32 rather than TF32 to favor
predictable first use, memory use and numerical consistency. These are conservative
configuration choices, not measured CUDA performance improvements. A matching
CUDA/cuDNN runtime and provider-enabled ONNX Runtime must be supplied. The
application does not download GPU drivers or change system library settings.
See [ONNX Runtime CUDA requirements](https://onnxruntime.ai/docs/execution-providers/CUDA-ExecutionProvider.html).

DirectML is the Windows path for compatible DirectX 12 GPUs, including supported
AMD and Intel hardware. Sessions use sequential execution and disabled memory
patterns as required by that provider. It also needs a DirectML-enabled runtime;
CPU fallback remains available. See [DirectML requirements](https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html).

Use `ORT_DYLIB_PATH` to select a compatible native runtime. Models and runtime must
match the app's ONNX API requirements. Selecting a backend in Settings cannot add
providers missing from that library. `--check-runtime` retains its CPU baseline;
`--check-runtime --runtime-backend coreml` (or `auto`, `cuda`, `directml`, `cpu`)
selects a diagnostic backend through the same production registry and fallback
policy. This uses the embedded licensed fixture and does not require a photo
library or change the saved backend preference.

Inspect `requested_backend` and all seven `model_providers` records in its JSON,
including fallback details. Native compiler/runtime output is routed to stderr
for the command lifetime, including shutdown, so stdout remains JSON. `passed` and exit status describe successful inference
and session reuse, allowing CPU fallback. `accelerated_models` and `cpu_models`
count configured sessions only after a complete passing check; they do not prove
per-operator GPU/NPU placement. A first-pass execution failure may replace one
accelerated session with CPU; no further session initialization is allowed during
the second pass. Partial or cancelled checks retain available provider diagnostics
but publish no successful provider counts. No prepared-model cache is configured
by this command; the native runtime may use OS temporary caches. Cold compilation
can take minutes, and a synchronous native compilation/inference call cannot be
interrupted. CUDA/DirectML hardware validation is still required on those devices.

## Persistent Core ML cache evaluation

An isolated October 6 probe of the pinned DINO/runtime measured median session
creation of 29.446 s without a provider cache versus 5.563 s when reused in fresh
processes (5.29x). All nine tested output buffers were identical. Each cached
model occupied about 398 MiB. Copying the cache to a fresh path raised session
creation to 26.802 s; a second load there took 5.610 s. Stable-path ownership,
model/runtime/recipe invalidation and integration with the shared disk budget
are therefore prerequisites. The source integration now uses these safeguards;
the probe's performance numbers remain isolated measurements, not integrated-app
benchmarks. See `diagnostics/coreml-cache-evaluation-2026-10-06/README.md`.


## Aesthetic model: folded offline instead of prepared per machine

Earlier builds kept a per-machine CPU-optimized copy of the aesthetic graph (about
510 MiB) because ONNX Runtime's constant folding of the 378 MB export peaked near
3.5 GiB on every uncached load. The bundled model is now folded once, offline, with
the hardware-independent basic optimizations (`examples/fold_model.rs`, outputs
bit-identical). A first load now peaks under 1 GiB and takes about 0.2 s, so the
per-machine CPU cache was removed; maintenance deletes old `.onnx-cache` entries.
Core ML program caching is unchanged. On the 517-photo sample library every
aesthetic score and Pick Best group is identical to the unfolded model at 8 and 4
threads. Back to back on the same machine, a fresh Quality analysis peaked at a
2.44 GiB memory footprint instead of 4.6 GiB (RSS 2.4–2.7 vs 3.6–4.2 GiB).

All seven models compile for Core ML. The face-landmarks export had pooling layers
with `auto_pad=VALID` and the blendshape export convolutions with no `pads`
attribute; both mean zero padding but the Core ML converter requires explicit pads,
so `scripts/explicit_pads.py` writes them (bit-identical on CPU). On the sample
library Core ML and CPU find the same 76 faces, the same closed-eye photos and the
same groups. A fresh Core ML run still pays one-time compilation, so CPU stays the
default.