#!/usr/bin/env python3
"""Compare two PCM WAV files for gain, timing, waveform, and tonal equivalence.

The script only needs NumPy. A positive reported lag means WAV B occurs later
than WAV A; comparisons compensate for that lag before calculating metrics.
"""

from __future__ import annotations

import argparse
import json
import math
import wave
from pathlib import Path

import numpy as np


def read_pcm_wav(path: Path) -> tuple[np.ndarray, int, int]:
    with wave.open(str(path), "rb") as wav:
        channels = wav.getnchannels()
        sample_width = wav.getsampwidth()
        sample_rate = wav.getframerate()
        frame_count = wav.getnframes()
        if wav.getcomptype() != "NONE":
            raise ValueError(f"Compressed WAV is unsupported: {path}")
        raw = wav.readframes(frame_count)

    if sample_width == 1:
        samples = (np.frombuffer(raw, dtype=np.uint8).astype(np.float32) - 128) / 128
    elif sample_width == 2:
        samples = np.frombuffer(raw, dtype="<i2").astype(np.float32) / 32768
    elif sample_width == 3:
        packed = np.frombuffer(raw, dtype=np.uint8).reshape(-1, 3)
        values = (
            packed[:, 0].astype(np.int32)
            | (packed[:, 1].astype(np.int32) << 8)
            | (packed[:, 2].astype(np.int32) << 16)
        )
        values = (values ^ 0x800000) - 0x800000
        samples = values.astype(np.float32) / 8388608
    elif sample_width == 4:
        samples = np.frombuffer(raw, dtype="<i4").astype(np.float32) / 2147483648
    else:
        raise ValueError(f"Unsupported PCM sample width: {sample_width} bytes")
    return samples.reshape(-1, channels), sample_rate, sample_width * 8


def db(value: float, floor: float = -300.0) -> float:
    return max(floor, 20.0 * math.log10(max(value, 10 ** (floor / 20))))


def rms(x: np.ndarray) -> float:
    return float(np.sqrt(np.mean(x.astype(np.float64) ** 2)))


def mono(x: np.ndarray) -> np.ndarray:
    return np.mean(x, axis=1, dtype=np.float32)


def fft_xcorr_lag(a: np.ndarray, b: np.ndarray, max_lag: int) -> int:
    """Return lag maximizing dot(a[t], b[t+lag])."""
    n = min(len(a), len(b))
    a = a[:n].astype(np.float64, copy=False)
    b = b[:n].astype(np.float64, copy=False)
    a = a - np.mean(a)
    b = b - np.mean(b)
    size = 1 << (2 * n - 1).bit_length()
    conv = np.fft.irfft(np.fft.rfft(b, size) * np.fft.rfft(a[::-1], size), size)
    lags = np.arange(-max_lag, max_lag + 1)
    indices = n - 1 + lags
    overlap = n - np.abs(lags)
    scores = conv[indices] / np.maximum(overlap, 1)
    return int(lags[int(np.argmax(scores))])


def coarse_lag(a: np.ndarray, b: np.ndarray, sample_rate: int, max_seconds: float) -> int:
    block = 64
    n = min(len(a), len(b)) // block * block
    # Absolute-amplitude envelopes remain useful even if the signal polarity differs.
    ea = np.mean(np.abs(mono(a[:n])).reshape(-1, block), axis=1)
    eb = np.mean(np.abs(mono(b[:n])).reshape(-1, block), axis=1)
    lag_blocks = fft_xcorr_lag(ea, eb, max(1, round(max_seconds * sample_rate / block)))
    return lag_blocks * block


def best_sample_lag(
    a: np.ndarray, b: np.ndarray, sample_rate: int, estimate: int, radius: int = 96
) -> int:
    """Refine a coarse lag using several energetic waveform excerpts."""
    am = mono(a)
    bm = mono(b)
    n = min(len(am), len(bm))
    window = min(sample_rate * 8, max(sample_rate, n // 8))
    starts = np.linspace(sample_rate, max(sample_rate, n - window - sample_rate), 9).astype(int)
    excerpts: list[tuple[np.ndarray, np.ndarray]] = []
    for start in starts:
        left = max(0, start)
        right = min(n, left + window)
        aa = am[left:right]
        if rms(aa) > 1e-5:
            excerpts.append((aa, bm[left:right]))
    if not excerpts:
        return estimate

    lags = range(estimate - radius, estimate + radius + 1)
    scores = []
    for lag in lags:
        total = 0.0
        norm = 0.0
        for aa, bb in excerpts:
            if lag >= 0:
                x, y = aa[: len(aa) - lag or None], bb[lag:]
            else:
                x, y = aa[-lag:], bb[: len(bb) + lag]
            total += float(np.dot(x.astype(np.float64), y.astype(np.float64)))
            norm += math.sqrt(float(np.dot(x, x)) * float(np.dot(y, y)))
        scores.append(total / max(norm, 1e-30))
    return int(list(lags)[int(np.argmax(scores))])


def aligned(a: np.ndarray, b: np.ndarray, lag: int) -> tuple[np.ndarray, np.ndarray]:
    if lag >= 0:
        return a[: min(len(a), len(b) - lag)], b[lag : lag + min(len(a), len(b) - lag)]
    return a[-lag : -lag + min(len(a) + lag, len(b))], b[: min(len(a) + lag, len(b))]


def write_pcm24(path: Path, samples: np.ndarray, sample_rate: int) -> None:
    clipped = np.clip(samples, -1.0, np.nextafter(np.float32(1.0), np.float32(0.0)))
    values = np.rint(clipped.astype(np.float64) * 8388608).astype(np.int32)
    unsigned = values.astype(np.uint32)
    packed = np.empty((values.size, 3), dtype=np.uint8)
    packed[:, 0] = unsigned.ravel() & 0xFF
    packed[:, 1] = (unsigned.ravel() >> 8) & 0xFF
    packed[:, 2] = (unsigned.ravel() >> 16) & 0xFF
    with wave.open(str(path), "wb") as wav:
        wav.setnchannels(samples.shape[1])
        wav.setsampwidth(3)
        wav.setframerate(sample_rate)
        wav.writeframes(packed.tobytes())


def local_lags(a: np.ndarray, b: np.ndarray, sample_rate: int, global_lag: int) -> list[dict]:
    results = []
    duration = min(len(a), len(b)) / sample_rate
    window = min(20.0, max(5.0, duration / 8))
    for center in np.linspace(window / 2, duration - window / 2, 7):
        start = int((center - window / 2) * sample_rate)
        end = int((center + window / 2) * sample_rate)
        aa, bb = mono(a[start:end]), mono(b[start:end])
        if rms(aa) < 1e-5 or rms(bb) < 1e-5:
            continue
        lag = fft_xcorr_lag(aa, bb, min(sample_rate // 10, abs(global_lag) + 2048))
        results.append({"time_seconds": round(center, 3), "lag_samples": lag})
    return results


def spectral_comparison(a: np.ndarray, b: np.ndarray, sample_rate: int) -> dict:
    frame = 8192
    hop = frame
    n = min(len(a), len(b))
    starts = np.arange(0, max(1, n - frame + 1), hop)
    # Cap work while sampling evenly across the complete recording.
    if len(starts) > 2048:
        starts = starts[np.linspace(0, len(starts) - 1, 2048).astype(int)]
    win = np.hanning(frame).astype(np.float32)
    pa = np.zeros(frame // 2 + 1, dtype=np.float64)
    pb = np.zeros_like(pa)
    used = 0
    for start in starts:
        xa = mono(a[start : start + frame])
        xb = mono(b[start : start + frame])
        if len(xa) != frame or max(rms(xa), rms(xb)) < 1e-6:
            continue
        pa += np.abs(np.fft.rfft(xa * win)) ** 2
        pb += np.abs(np.fft.rfft(xb * win)) ** 2
        used += 1
    pa /= max(used, 1)
    pb /= max(used, 1)
    freqs = np.fft.rfftfreq(frame, 1 / sample_rate)
    bands = [(20, 60), (60, 250), (250, 1000), (1000, 4000), (4000, 10000), (10000, 20000)]
    band_delta = {}
    band_levels = {}
    window_energy = float(np.sum(win.astype(np.float64) ** 2))
    for low, high in bands:
        mask = (freqs >= low) & (freqs < min(high, sample_rate / 2))
        if np.any(mask):
            power_a = float(np.sum(pa[mask]))
            power_b = float(np.sum(pb[mask]))
            ratio = math.sqrt(power_b / max(power_a, 1e-30))
            name = f"{low}-{high}_Hz"
            band_delta[name] = round(db(ratio), 4)
            # One-sided FFT energy, excluding DC/Nyquist in these bands.
            mean_square_a = 2 * power_a / (frame * window_energy)
            mean_square_b = 2 * power_b / (frame * window_energy)
            band_levels[name] = {
                "A_dBFS": round(db(math.sqrt(mean_square_a)), 3),
                "B_dBFS": round(db(math.sqrt(mean_square_b)), 3),
            }
    valid = (freqs >= 20) & (freqs <= min(20000, sample_rate / 2)) & (pa > np.max(pa) * 1e-10)
    curve = 10 * np.log10(np.maximum(pb[valid], 1e-30) / np.maximum(pa[valid], 1e-30))
    return {
        "frames_used": used,
        "band_level_B_minus_A_dB": band_delta,
        "band_absolute_levels": band_levels,
        "spectral_delta_std_dB": round(float(np.std(curve)), 4) if len(curve) else None,
        "spectral_delta_max_abs_dB": round(float(np.max(np.abs(curve))), 4) if len(curve) else None,
    }


def channel_metrics(a: np.ndarray, b: np.ndarray) -> list[dict]:
    output = []
    labels = ["L", "R"] if a.shape[1] == 2 else [f"ch{i + 1}" for i in range(a.shape[1])]
    for index, label in enumerate(labels):
        x = a[:, index].astype(np.float64)
        y = b[:, index].astype(np.float64)
        gain = float(np.dot(x, y) / max(float(np.dot(x, x)), 1e-30))
        error = y - gain * x
        corr = float(np.dot(x, y) / max(math.sqrt(float(np.dot(x, x)) * float(np.dot(y, y))), 1e-30))
        output.append(
            {
                "channel": label,
                "A_rms_dBFS": round(db(rms(x)), 4),
                "B_rms_dBFS": round(db(rms(y)), 4),
                "A_peak_dBFS": round(db(float(np.max(np.abs(x)))), 4),
                "B_peak_dBFS": round(db(float(np.max(np.abs(y)))), 4),
                "gain_B_over_A_dB": round(db(abs(gain)), 5),
                "polarity_inverted": gain < 0,
                "correlation": round(corr, 9),
                "gain_matched_residual_rms_dBFS": round(db(rms(error)), 4),
                "gain_matched_residual_peak_dBFS": round(db(float(np.max(np.abs(error)))), 4),
                "gain_matched_null_dB_relative_to_B": round(db(rms(error) / max(rms(y), 1e-30)), 4),
            }
        )
    return output


def compare(
    path_a: Path,
    path_b: Path,
    max_lag_seconds: float,
    null_wav: Path | None = None,
    audition_null_wav: Path | None = None,
    audition_gain_db: float = 50.0,
) -> dict:
    a, rate_a, bits_a = read_pcm_wav(path_a)
    b, rate_b, bits_b = read_pcm_wav(path_b)
    if rate_a != rate_b or a.shape[1] != b.shape[1]:
        raise ValueError("Sample rates and channel counts must match")

    estimate = coarse_lag(a, b, rate_a, max_lag_seconds)
    lag = best_sample_lag(a, b, rate_a, estimate)
    drift = local_lags(a, b, rate_a, lag)
    aa, bb = aligned(a, b, lag)
    raw_null = bb - aa
    if null_wav:
        write_pcm24(null_wav, raw_null, rate_a)
    if audition_null_wav:
        write_pcm24(audition_null_wav, raw_null * (10 ** (audition_gain_db / 20)), rate_a)
    return {
        "file_A": str(path_a.resolve()),
        "file_B": str(path_b.resolve()),
        "format": {"sample_rate": rate_a, "channels": a.shape[1], "bits": [bits_a, bits_b]},
        "duration_seconds": {"A": len(a) / rate_a, "B": len(b) / rate_a, "aligned": len(aa) / rate_a},
        "length_difference_samples_B_minus_A": len(b) - len(a),
        "alignment": {
            "lag_samples_B_after_A": lag,
            "lag_milliseconds": lag / rate_a * 1000,
            "local_lags": drift,
            "local_lag_span_samples": (max(x["lag_samples"] for x in drift) - min(x["lag_samples"] for x in drift)) if drift else None,
        },
        "raw_null": {
            "operation": "B minus A after integer-sample alignment; no gain or EQ matching",
            "channel_rms_dBFS": [round(db(rms(raw_null[:, i])), 4) for i in range(raw_null.shape[1])],
            "channel_peak_dBFS": [round(db(float(np.max(np.abs(raw_null[:, i])))), 4) for i in range(raw_null.shape[1])],
            "wav": str(null_wav.resolve()) if null_wav else None,
            "audition_wav": str(audition_null_wav.resolve()) if audition_null_wav else None,
            "audition_gain_dB": audition_gain_db if audition_null_wav else None,
        },
        "channels": channel_metrics(aa, bb),
        "spectrum": spectral_comparison(aa, bb, rate_a),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("wav_a", type=Path, help="reference/native WAV")
    parser.add_argument("wav_b", type=Path, help="comparison/loopback WAV")
    parser.add_argument("--max-lag-seconds", type=float, default=2.0)
    parser.add_argument("--json", type=Path, help="also save the report as JSON")
    parser.add_argument("--null-wav", type=Path, help="write the aligned, unmodified B-minus-A null")
    parser.add_argument("--audition-null-wav", type=Path, help="write an amplified null for listening")
    parser.add_argument("--audition-gain-db", type=float, default=50.0)
    args = parser.parse_args()
    result = compare(
        args.wav_a,
        args.wav_b,
        args.max_lag_seconds,
        args.null_wav,
        args.audition_null_wav,
        args.audition_gain_db,
    )
    rendered = json.dumps(result, ensure_ascii=False, indent=2)
    print(rendered)
    if args.json:
        args.json.write_text(rendered + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
