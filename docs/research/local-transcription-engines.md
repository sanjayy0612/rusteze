# Local transcription engine decision

## Recommendation

Use **`whisper.cpp`** as Rusteze's first local transcription runtime, with
OpenAI **Whisper `large-v3-turbo`** as the default downloadable model. Offer
`small.en` (English-only) and `small` (multilingual) as lower-memory options;
keep `large-v3` opt-in for accuracy-first users.

This is the best integration fit for an offline Rust/macOS product, rather than a
claim that one model is universally most accurate. Before locking the default,
benchmark 10–20 consented Rusteze recordings representing accents, terminology,
noise, crosstalk, and meeting lengths. Score word error rate, names/terms, elapsed
time, and peak memory.

## Why `whisper.cpp`

`whisper.cpp` has a dependency-light C/C++ implementation and C-style API. Its
official project calls Apple Silicon a first-class target, with NEON, Accelerate,
Metal, and Core ML; it also supports quantization and VAD. The project says Metal
can run inference fully on the Apple Silicon GPU. This permits either a subprocess
adapter first or a later Rust FFI adapter, without shipping Python.

The bundled CLI currently accepts 16-bit WAV input. Rusteze should therefore
convert each completed CAF track to 16-kHz mono PCM WAV (or use the C API), while
retaining the original separate CAF recordings.

Sources: [whisper.cpp README](https://github.com/ggml-org/whisper.cpp/blob/master/README.md),
[model instructions](https://github.com/ggml-org/whisper.cpp/blob/master/models/README.md).

## Model tiers

| Need | Model | Rationale |
|---|---|---|
| Default quality/speed | `large-v3-turbo` | Multilingual and optimized for inference speed with minimal accuracy degradation. |
| Lower-memory English-only | `small.en` | Official Whisper docs say `.en` variants tend to perform better for English-only use. |
| Lower-memory multilingual | `small` | Preserves multilingual transcription. |
| Accuracy-first | `large-v3` | Full large model; validate its cost/benefit with the local corpus. |

OpenAI lists `turbo` at about 809M parameters versus 1.55B for `large`; it is not
the translation model, so use multilingual `medium` or `large` when translating
speech into English. Whisper supports multilingual ASR, language ID, and speech
translation, but its model card says diarization has not been robustly evaluated.
The `whisper.cpp` Core ML encoder is an optional optimization: its documented
greater-than-3x speed-up is versus CPU-only and is hardware-dependent.

Sources: [OpenAI Whisper README](https://github.com/openai/whisper#available-models-and-languages),
[OpenAI model card](https://github.com/openai/whisper/blob/main/model-card.md),
[large-v3-turbo card](https://huggingface.co/openai/whisper-large-v3-turbo),
[Core ML instructions](https://github.com/ggml-org/whisper.cpp#core-ml-support).

## Alternatives

| Engine | Assessment | Decision |
|---|---|---|
| `whisper.cpp` | Native API and explicit Apple Silicon support. | **Use first.** |
| `mlx-whisper` | Apple Silicon-focused and supports word timestamps/quantized conversion, but its official path is Python, `ffmpeg`, and MLX checkpoints. | Experimental future backend. |
| `faster-whisper` | Useful Python/CTranslate2 engine with VAD and word timestamps; official docs document CUDA and CPU paths, not native Apple-Silicon/MPS. | Not first macOS runtime. |
| OpenAI `whisper` | Canonical Python implementation, requiring Python/PyTorch dependencies. | Model/reference source, not runtime. |
| NVIDIA Parakeet TDT 0.6B v3 | Has punctuation and timestamps, but its official runtime guidance targets NeMo on Linux/NVIDIA GPUs. | Revisit only for a future NVIDIA target. |

Sources: [MLX](https://github.com/ml-explore/mlx), [MLX Whisper](https://github.com/ml-explore/mlx-examples/blob/main/whisper/README.md),
[faster-whisper](https://github.com/SYSTRAN/faster-whisper), [OpenAI Whisper setup](https://github.com/openai/whisper#setup),
[NVIDIA Parakeet model card](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3).

## Recording-specific boundary

Transcribe `mic.caf` and `system.caf` independently. Preserve each segment's time
range and `track`, then merge by time for the transcript files. Tracks distinguish
local microphone from remote/system audio; they do not diarize individual remote
speakers. Keep diarization as a later, separate capability.

Implementation: add `WhisperCppEngine` behind the existing trait; make download
explicit, checksummed, versioned, and never during recording; convert completed
audio in a per-session temporary location; then benchmark before finalizing the
default model or enabling Core ML/quantization by default.
