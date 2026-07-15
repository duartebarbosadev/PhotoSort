"""Cross-platform resource detection and thumbnail worker policy tests."""

import io
from types import SimpleNamespace

from core import app_settings


def _set_policy_inputs(monkeypatch, *, cpus, memory_gib, mode, custom=None):
    monkeypatch.setattr(app_settings, "get_available_cpu_count", lambda: cpus)
    monkeypatch.setattr(
        app_settings, "get_usable_memory_bytes", lambda: memory_gib * 1024**3
    )
    monkeypatch.setattr(app_settings, "get_performance_mode", lambda: mode)
    if custom is not None:
        monkeypatch.setattr(app_settings, "get_custom_thread_count", lambda: custom)


def test_performance_policy_scales_up_on_large_machine(monkeypatch):
    _set_policy_inputs(
        monkeypatch,
        cpus=32,
        memory_gib=64,
        mode=app_settings.PerformanceMode.PERFORMANCE,
    )

    assert app_settings.calculate_thumbnail_workers() == 32
    assert app_settings.calculate_high_memory_decode_workers() == 8


def test_performance_policy_scales_down_on_low_memory_machine(monkeypatch):
    _set_policy_inputs(
        monkeypatch,
        cpus=16,
        memory_gib=4,
        mode=app_settings.PerformanceMode.PERFORMANCE,
    )

    assert app_settings.calculate_thumbnail_workers() == 4
    assert app_settings.calculate_high_memory_decode_workers() == 1


def test_balanced_policy_leaves_more_capacity_for_other_work(monkeypatch):
    _set_policy_inputs(
        monkeypatch,
        cpus=32,
        memory_gib=64,
        mode=app_settings.PerformanceMode.BALANCED,
    )

    assert app_settings.calculate_thumbnail_workers() == 27
    assert app_settings.calculate_high_memory_decode_workers() == 4


def test_custom_policy_respects_thread_and_memory_limits(monkeypatch):
    _set_policy_inputs(
        monkeypatch,
        cpus=24,
        memory_gib=16,
        mode=app_settings.PerformanceMode.CUSTOM,
        custom=6,
    )

    assert app_settings.calculate_thumbnail_workers() == 6
    assert app_settings.calculate_high_memory_decode_workers() == 2


def test_cpu_detection_prefers_process_limit(monkeypatch):
    monkeypatch.setattr(app_settings.os, "process_cpu_count", lambda: 6, raising=False)
    monkeypatch.setattr(app_settings.os, "cpu_count", lambda: 24)

    assert app_settings.get_available_cpu_count() == 6


def test_cpu_detection_uses_linux_affinity_when_available(monkeypatch):
    monkeypatch.setattr(app_settings.os, "process_cpu_count", None, raising=False)
    monkeypatch.setattr(
        app_settings.os,
        "sched_getaffinity",
        lambda _pid: {2, 3, 4},
        raising=False,
    )
    monkeypatch.setattr(app_settings.os, "cpu_count", lambda: 24)

    assert app_settings.get_available_cpu_count() == 3


def test_windows_memory_detection_uses_global_memory_status(monkeypatch):
    expected = 16 * 1024**3

    def global_memory_status(status_pointer):
        status_pointer._obj.ullTotalPhys = expected
        return 1

    kernel32 = SimpleNamespace(GlobalMemoryStatusEx=global_memory_status)
    monkeypatch.setattr(app_settings.os, "name", "nt")
    monkeypatch.setattr(
        app_settings.ctypes,
        "windll",
        SimpleNamespace(kernel32=kernel32),
        raising=False,
    )

    assert app_settings.get_total_physical_memory_bytes() == expected


def test_linux_container_memory_limit_caps_host_memory(monkeypatch):
    monkeypatch.setattr(app_settings.sys, "platform", "linux")
    monkeypatch.setattr(
        app_settings, "get_total_physical_memory_bytes", lambda: 64 * 1024**3
    )

    def open_limit(path, **_kwargs):
        if path.endswith("memory.max"):
            return io.StringIO(str(6 * 1024**3))
        raise OSError

    monkeypatch.setattr("builtins.open", open_limit)

    assert app_settings.get_usable_memory_bytes() == 6 * 1024**3
