import hashlib
import json
import math
import os
import time
import wave
from pathlib import Path
from typing import Any

import numpy as np
from PIL import Image

from bobe_hil.paths import sha256_file


MEDIA_SCHEMA = 1
PCM16_MAX = 32767
MAX_PLAYBACK_PEAK = PCM16_MAX // 4
MAX_PLAYBACK_SECONDS = 10.0
MAX_AUDIO_SAMPLES = 3_000_000
MAX_IMAGE_PIXELS = 8_388_608


def checked_media_file(path: Path, description: str) -> Path:
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"{description} must be a regular non-symlink file")
    return path


def write_wav(path: Path, samples: np.ndarray, sample_rate: int) -> None:
    if path.is_symlink() or path.exists():
        raise ValueError("WAV output must be a new non-symlink path")
    if samples.dtype != np.int16 or samples.ndim != 1:
        raise ValueError("WAV samples must be mono PCM16")
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    descriptor = os.open(
        path,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
        0o600,
    )
    with os.fdopen(descriptor, "wb") as raw_output:
        with wave.open(raw_output, "wb") as output:
            output.setnchannels(1)
            output.setsampwidth(2)
            output.setframerate(sample_rate)
            output.writeframes(samples.astype("<i2", copy=False).tobytes())


def read_wav(path: Path) -> tuple[np.ndarray, int]:
    checked_media_file(path, "WAV input")
    try:
        with wave.open(str(path), "rb") as source:
            if source.getnchannels() != 1 or source.getsampwidth() != 2:
                raise ValueError("WAV input must be mono PCM16")
            sample_rate = source.getframerate()
            frame_count = source.getnframes()
            if not 8000 <= sample_rate <= 96000:
                raise ValueError("WAV sample rate is unsupported")
            if frame_count <= 0 or frame_count > MAX_AUDIO_SAMPLES:
                raise ValueError("WAV sample count is outside the bounded range")
            frames = source.readframes(frame_count)
    except (EOFError, wave.Error) as error:
        raise ValueError("WAV input is malformed") from error
    return np.frombuffer(frames, dtype="<i2").astype(np.int16), sample_rate


def validate_playback_stimulus(
    samples: np.ndarray,
    sample_rate: int,
) -> None:
    if len(samples) == 0:
        raise ValueError("audio playback stimulus is empty")
    duration = len(samples) / sample_rate
    peak = int(np.max(np.abs(samples.astype(np.int32))))
    if duration > MAX_PLAYBACK_SECONDS:
        raise ValueError("audio playback stimulus exceeds 10 seconds")
    if peak > MAX_PLAYBACK_PEAK:
        raise ValueError("audio playback stimulus exceeds the safe peak bound")


def audio_challenge(
    nonce: str,
    sample_rate: int = 48000,
    duration_seconds: float = 3.0,
) -> np.ndarray:
    if len(nonce) != 32 or any(character not in "0123456789abcdef" for character in nonce):
        raise ValueError("audio challenge nonce is malformed")
    if not 8000 <= sample_rate <= 96000:
        raise ValueError("audio challenge sample rate is unsupported")
    if not 0.5 <= duration_seconds <= 10.0:
        raise ValueError("audio challenge duration is outside the bounded range")
    count = round(sample_rate * duration_seconds)
    time = np.arange(count, dtype=np.float64) / sample_rate
    seed = int(hashlib.sha256(f"audio:{nonce}".encode()).hexdigest()[:16], 16)
    start_hz = 350 + seed % 251
    end_hz = 3200 + (seed >> 8) % 1201
    sweep_rate = (end_hz - start_hz) / duration_seconds
    phase = 2 * math.pi * (
        start_hz * time + 0.5 * sweep_rate * np.square(time)
    )
    envelope_samples = max(1, round(sample_rate * 0.02))
    envelope = np.ones(count, dtype=np.float64)
    ramp = np.linspace(0.0, 1.0, envelope_samples, endpoint=False)
    envelope[:envelope_samples] = ramp
    envelope[-envelope_samples:] = ramp[::-1]
    signal = 0.18 * np.sin(phase) * envelope
    return np.clip(np.rint(signal * PCM16_MAX), -PCM16_MAX, PCM16_MAX).astype(
        np.int16
    )


def next_power_of_two(value: int) -> int:
    return 1 << (value - 1).bit_length()


def positive_cross_correlation(
    reference: np.ndarray,
    capture: np.ndarray,
) -> tuple[int, float]:
    reference_float = reference.astype(np.float64)
    capture_float = capture.astype(np.float64)
    reference_float -= reference_float.mean()
    capture_float -= capture_float.mean()
    fft_size = next_power_of_two(len(reference_float) + len(capture_float) - 1)
    correlation = np.fft.irfft(
        np.fft.rfft(capture_float, fft_size)
        * np.conj(np.fft.rfft(reference_float, fft_size)),
        fft_size,
    )
    positive = correlation[: len(capture_float)]
    lag = int(np.argmax(positive))
    overlap = min(len(reference_float), len(capture_float) - lag)
    if overlap <= 0:
        return lag, 0.0
    left = reference_float[:overlap]
    right = capture_float[lag : lag + overlap]
    denominator = float(np.linalg.norm(left) * np.linalg.norm(right))
    coefficient = float(np.dot(left, right) / denominator) if denominator else 0.0
    return lag, coefficient


def audio_metrics(reference_path: Path, capture_path: Path) -> dict[str, Any]:
    reference, reference_rate = read_wav(reference_path)
    capture, capture_rate = read_wav(capture_path)
    if reference_rate != capture_rate:
        raise ValueError("reference and capture sample rates differ")
    if len(reference) == 0 or len(capture) == 0:
        raise ValueError("audio input is empty")
    lag, correlation = positive_cross_correlation(reference, capture)
    capture_float = capture.astype(np.float64)
    absolute = np.abs(capture_float)
    frame_samples = max(1, reference_rate // 50)
    complete = len(capture_float) // frame_samples
    frames = capture_float[: complete * frame_samples].reshape(
        complete,
        frame_samples,
    )
    frame_rms = (
        np.sqrt(np.mean(np.square(frames), axis=1))
        if complete
        else np.array([], dtype=np.float64)
    )
    overall_rms = float(np.sqrt(np.mean(np.square(capture_float))))
    dropout_threshold = max(20.0, overall_rms * 0.03)
    dropout_frames = int(np.count_nonzero(frame_rms < dropout_threshold))
    return {
        "schema_version": MEDIA_SCHEMA,
        "sample_rate": reference_rate,
        "reference_samples": int(len(reference)),
        "capture_samples": int(len(capture)),
        "reference_sha256": sha256_file(reference_path),
        "capture_sha256": sha256_file(capture_path),
        "peak": int(absolute.max()),
        "rms": overall_rms,
        "dc_offset": float(capture_float.mean()),
        "clipped_samples": int(np.count_nonzero(absolute >= PCM16_MAX)),
        "dropout_frames_20ms": dropout_frames,
        "latency_samples": lag,
        "latency_ms": lag * 1000.0 / reference_rate,
        "normalized_correlation": correlation,
    }


def image_metrics(expected_path: Path, actual_path: Path) -> dict[str, Any]:
    checked_media_file(expected_path, "expected image")
    checked_media_file(actual_path, "actual image")
    try:
        with Image.open(expected_path) as expected_image:
            if (
                expected_image.width > 4096
                or expected_image.height > 4096
                or expected_image.width * expected_image.height
                > MAX_IMAGE_PIXELS
            ):
                raise ValueError("expected image dimensions exceed bounds")
            expected = np.asarray(
                expected_image.convert("RGB"),
                dtype=np.int16,
            )
        with Image.open(actual_path) as actual_image:
            if (
                actual_image.width > 4096
                or actual_image.height > 4096
                or actual_image.width * actual_image.height
                > MAX_IMAGE_PIXELS
            ):
                raise ValueError("actual image dimensions exceed bounds")
            actual = np.asarray(actual_image.convert("RGB"), dtype=np.int16)
    except (Image.DecompressionBombError, OSError) as error:
        raise ValueError("image input is malformed or unsafe") from error
    if expected.shape != actual.shape:
        raise ValueError(
            f"image dimensions differ: {expected.shape[:2]} != {actual.shape[:2]}"
        )
    delta = np.abs(expected - actual)
    squared = np.square(expected.astype(np.float64) - actual.astype(np.float64))
    pixels = expected.shape[0] * expected.shape[1]
    mismatched_pixels = int(np.count_nonzero(np.any(delta != 0, axis=2)))
    return {
        "schema_version": MEDIA_SCHEMA,
        "width": expected.shape[1],
        "height": expected.shape[0],
        "expected_sha256": sha256_file(expected_path),
        "actual_sha256": sha256_file(actual_path),
        "max_channel_error": int(delta.max()),
        "mean_absolute_channel_error": float(delta.mean()),
        "root_mean_square_channel_error": float(np.sqrt(squared.mean())),
        "mismatched_pixels": mismatched_pixels,
        "mismatched_pixel_ratio": mismatched_pixels / pixels,
    }


def audio_device_list() -> list[dict[str, Any]]:
    import sounddevice

    try:
        host_apis = sounddevice.query_hostapis()
        return [
            {
                "index": index,
                "name": device["name"],
                "host_api": host_apis[device["hostapi"]]["name"],
                "input_channels": device["max_input_channels"],
                "output_channels": device["max_output_channels"],
                "default_sample_rate": device["default_samplerate"],
            }
            for index, device in enumerate(sounddevice.query_devices())
        ]
    except sounddevice.PortAudioError as error:
        raise RuntimeError("CoreAudio device discovery failed") from error


def resolve_audio_device(name: str, direction: str) -> int:
    if direction not in {"input", "output"}:
        raise ValueError("audio device direction is invalid")
    channel_key = f"{direction}_channels"
    matches = [
        device
        for device in audio_device_list()
        if device["name"] == name and int(device[channel_key]) > 0
    ]
    if len(matches) != 1:
        raise ValueError(
            f"expected exactly one {direction} audio device named {name!r}, "
            f"found {len(matches)}"
        )
    return int(matches[0]["index"])


def audio_device_fingerprint(index: int) -> dict[str, Any]:
    devices = {
        int(device["index"]): device for device in audio_device_list()
    }
    if index not in devices:
        raise RuntimeError("CoreAudio device disappeared")
    return devices[index]


def record_audio(
    device_name: str,
    output_path: Path,
    duration_seconds: float,
    sample_rate: int,
) -> dict[str, Any]:
    import sounddevice

    if not 0.1 <= duration_seconds <= 30.0:
        raise ValueError("audio record duration is outside the bounded range")
    if not 8000 <= sample_rate <= 96000:
        raise ValueError("audio record sample rate is unsupported")
    if output_path.exists() or output_path.is_symlink():
        raise ValueError("audio record output must be a new non-symlink path")
    device_index = resolve_audio_device(device_name, "input")
    device_fingerprint = audio_device_fingerprint(device_index)
    blocks: list[np.ndarray] = []
    status_messages: list[str] = []

    def callback(
        data: np.ndarray,
        frames: int,
        timing: object,
        status: object,
    ) -> None:
        del frames, timing
        if status:
            status_messages.append(str(status))
        blocks.append(data[:, 0].copy())

    try:
        with sounddevice.InputStream(
            samplerate=sample_rate,
            device=device_index,
            channels=1,
            dtype="float32",
            callback=callback,
        ) as stream:
            actual_sample_rate = int(stream.samplerate)
            time.sleep(duration_seconds)
    except sounddevice.PortAudioError as error:
        raise RuntimeError("CoreAudio capture failed") from error
    if audio_device_fingerprint(device_index) != device_fingerprint:
        raise RuntimeError("CoreAudio input identity changed during capture")
    if not blocks:
        raise RuntimeError("CoreAudio capture returned no samples")
    samples_float = np.concatenate(blocks)
    expected_samples = round(duration_seconds * sample_rate)
    samples_float = samples_float[:expected_samples]
    samples = np.clip(
        np.rint(samples_float * PCM16_MAX),
        -PCM16_MAX,
        PCM16_MAX,
    ).astype(np.int16)
    write_wav(output_path, samples, sample_rate)
    completeness = len(samples) / expected_samples
    return {
        "schema_version": MEDIA_SCHEMA,
        "device_name": device_name,
        "device_index": device_index,
        "device": device_fingerprint,
        "sample_rate": sample_rate,
        "actual_sample_rate": actual_sample_rate,
        "duration_seconds": duration_seconds,
        "captured_duration_seconds": len(samples) / sample_rate,
        "sample_count": int(len(samples)),
        "expected_sample_count": expected_samples,
        "sample_completeness": completeness,
        "status_messages": status_messages,
        "clean_stream": not status_messages and completeness >= 0.95,
        "output": str(output_path),
        "sha256": sha256_file(output_path),
    }


def play_audio(device_name: str, input_path: Path) -> dict[str, Any]:
    import sounddevice

    samples, sample_rate = read_wav(input_path)
    validate_playback_stimulus(samples, sample_rate)
    device_index = resolve_audio_device(device_name, "output")
    device_fingerprint = audio_device_fingerprint(device_index)
    normalized = samples.astype(np.float32) / PCM16_MAX
    try:
        sounddevice.play(
            normalized,
            samplerate=sample_rate,
            device=device_index,
            blocking=True,
        )
        stream_status = str(sounddevice.get_status())
    except sounddevice.PortAudioError as error:
        raise RuntimeError("CoreAudio playback failed") from error
    if audio_device_fingerprint(device_index) != device_fingerprint:
        raise RuntimeError("CoreAudio output identity changed during playback")
    return {
        "schema_version": MEDIA_SCHEMA,
        "device_name": device_name,
        "device_index": device_index,
        "device": device_fingerprint,
        "sample_rate": sample_rate,
        "sample_count": int(len(samples)),
        "input": str(input_path),
        "sha256": sha256_file(input_path),
        "status": stream_status,
        "clean_stream": not stream_status,
    }


def play_record_audio(
    input_device_name: str,
    output_device_name: str,
    stimulus_path: Path,
    capture_path: Path,
    tail_seconds: float,
) -> dict[str, Any]:
    import sounddevice

    if not 0.0 <= tail_seconds <= 5.0:
        raise ValueError("audio roundtrip tail is outside the bounded range")
    if capture_path.exists() or capture_path.is_symlink():
        raise ValueError("audio roundtrip output must be a new non-symlink path")
    stimulus, sample_rate = read_wav(stimulus_path)
    validate_playback_stimulus(stimulus, sample_rate)
    input_index = resolve_audio_device(input_device_name, "input")
    output_index = resolve_audio_device(output_device_name, "output")
    input_fingerprint = audio_device_fingerprint(input_index)
    output_fingerprint = audio_device_fingerprint(output_index)
    tail_samples = round(tail_seconds * sample_rate)
    playback = np.concatenate(
        (stimulus, np.zeros(tail_samples, dtype=np.int16))
    )
    normalized = playback.astype(np.float32) / PCM16_MAX
    try:
        captured = sounddevice.playrec(
            normalized,
            samplerate=sample_rate,
            channels=1,
            dtype="float32",
            device=(input_index, output_index),
            blocking=True,
        )
        stream_status = str(sounddevice.get_status())
    except sounddevice.PortAudioError as error:
        raise RuntimeError("CoreAudio roundtrip failed") from error
    if (
        audio_device_fingerprint(input_index) != input_fingerprint
        or audio_device_fingerprint(output_index) != output_fingerprint
    ):
        raise RuntimeError("CoreAudio identity changed during roundtrip")
    if captured.ndim != 2 or captured.shape != (len(playback), 1):
        raise RuntimeError("CoreAudio roundtrip returned an unexpected shape")
    samples = np.clip(
        np.rint(captured[:, 0] * PCM16_MAX),
        -PCM16_MAX,
        PCM16_MAX,
    ).astype(np.int16)
    write_wav(capture_path, samples, sample_rate)
    return {
        "schema_version": MEDIA_SCHEMA,
        "input_device_name": input_device_name,
        "input_device_index": input_index,
        "input_device": input_fingerprint,
        "output_device_name": output_device_name,
        "output_device_index": output_index,
        "output_device": output_fingerprint,
        "sample_rate": sample_rate,
        "stimulus_samples": int(len(stimulus)),
        "capture_samples": int(len(samples)),
        "tail_seconds": tail_seconds,
        "status": stream_status,
        "clean_stream": (
            len(samples) == len(playback) and not stream_status
        ),
        "stimulus_sha256": sha256_file(stimulus_path),
        "capture_sha256": sha256_file(capture_path),
        "capture": str(capture_path),
    }


def rgb565_to_png(
    source_path: Path,
    output_path: Path,
    width: int,
    height: int,
) -> None:
    checked_media_file(source_path, "RGB565 input")
    if width <= 0 or height <= 0 or width > 4096 or height > 4096:
        raise ValueError("RGB565 dimensions are outside the bounded range")
    data = source_path.read_bytes()
    if len(data) != width * height * 2:
        raise ValueError("RGB565 input size does not match dimensions")
    if output_path.is_symlink() or output_path.exists():
        raise ValueError("PNG output must be a new non-symlink path")
    pixels = np.frombuffer(data, dtype=">u2").reshape(height, width)
    red5 = (pixels >> 11) & 0x1F
    green6 = (pixels >> 5) & 0x3F
    blue5 = pixels & 0x1F
    rgb = np.stack(
        (
            (red5 << 3) | (red5 >> 2),
            (green6 << 2) | (green6 >> 4),
            (blue5 << 3) | (blue5 >> 2),
        ),
        axis=2,
    ).astype(np.uint8)
    output_path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    descriptor = os.open(
        output_path,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
        0o600,
    )
    try:
        with os.fdopen(descriptor, "wb") as output:
            Image.fromarray(rgb, "RGB").save(
                output,
                format="PNG",
                optimize=False,
            )
    except (OSError, ValueError):
        output_path.unlink(missing_ok=True)
        raise


def write_metrics(path: Path | None, metrics: dict[str, Any]) -> None:
    rendered = json.dumps(metrics, indent=2, sort_keys=True) + "\n"
    if path is None:
        print(rendered, end="")
        return
    if path.is_symlink() or path.exists():
        raise ValueError("metrics output must be a new non-symlink path")
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    descriptor = os.open(
        path,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
        0o600,
    )
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            output.write(rendered)
    except OSError:
        path.unlink(missing_ok=True)
        raise
