// Конфігурація й живий знімок метрик для UI-тестів «Заліза».
import type { HardwareInfo } from "../../src/lib/bindings/HardwareInfo";
import type { Snapshot } from "../../src/lib/bindings/Snapshot";

const GIB = 1024 ** 3;

export const hwInfo: HardwareInfo = {
  cpu_name: "AMD Ryzen 7 5800X3D 8-Core Processor",
  logical_cores: 16,
  physical_cores: 8,
  firmware: {
    bios: { vendor: "American Megatrends", version: "F37d", release_date: { year: 2024, month: 3, day: 12 } },
    memory: [
      {
        locator: "DIMM_A2",
        bank: "BANK 1",
        size_mib: 16384,
        memory_type: "ddr4",
        rated_mts: 3600,
        configured_mts: 3600,
        manufacturer: "G.Skill",
        part_number: "F4-3600C16-16GVKC",
      },
      {
        locator: "DIMM_B2",
        bank: "BANK 3",
        size_mib: 16384,
        memory_type: "ddr4",
        rated_mts: 3600,
        configured_mts: 2133,
        manufacturer: "G.Skill",
        part_number: "F4-3600C16-16GVKC",
      },
    ],
    memory_slots: 4,
  },
  displays: [
    { device_name: "\\\\.\\DISPLAY1", monitor_name: "LG 27GP850", primary: true, width: 2560, height: 1440, current_hz: 165, max_hz_at_current_res: 180 },
    { device_name: "\\\\.\\DISPLAY2", monitor_name: "DELL U2419H", primary: false, width: 1920, height: 1080, current_hz: 60, max_hz_at_current_res: 60 },
  ],
  gpus: [
    {
      name: "NVIDIA GeForce RTX 4070",
      vendor: "nvidia",
      driver_version: "32.0.15.6636",
      driver_date: { year: 2026, month: 4, day: 26 },
      pcie: { current_width: 16, max_width: 16, current_gen: 4, max_gen: 4 },
      rebar: { state: "enabled", bar1_mib: 16384 },
      vram_mib: 12282,
    },
  ],
};

export const hwSnap: Snapshot = {
  elapsed_ms: 500,
  qpc: 0,
  cpu: {
    total_percent: 31.4,
    per_core_percent: [97, 41, 22, 18, 64, 12, 9, 33, 28, 15, 11, 7, 52, 19, 8, 6],
    per_core_mhz: Array(16).fill(4450),
    dpc_percent: 0.4,
    temp_c: 71,
  },
  memory: { total_bytes: 32 * GIB, used_bytes: 18.6 * GIB, available_bytes: 13.4 * GIB, hard_fault_pages_per_sec: 0 },
  disk: { read_bytes_per_sec: 12.4 * 1048576, write_bytes_per_sec: 1.8 * 1048576 },
  gpus: [
    {
      index: 0,
      name: "NVIDIA GeForce RTX 4070",
      utilization_percent: 96,
      vram_used_bytes: 9.8 * GIB,
      vram_total_bytes: 12 * GIB,
      core_clock_mhz: 2715,
      max_core_clock_mhz: 3105,
      memory_clock_mhz: 10501,
      temperature_c: 68,
      power_w: 196,
      power_limit_w: 200,
      default_power_limit_w: 200,
      fan_percent: 54,
      throttle: ["sw_power_cap"],
    },
  ],
  top_processes: [
    { pid: 4120, name: "re9.exe", cpu_percent: 24.8, memory_bytes: 9.1 * GIB, io_bytes_per_sec: 8 * 1048576 },
    { pid: 812, name: "chrome.exe", cpu_percent: 3.1, memory_bytes: 1.4 * GIB, io_bytes_per_sec: 0 },
    { pid: 2280, name: "Discord.exe", cpu_percent: 1.2, memory_bytes: 0.6 * GIB, io_bytes_per_sec: 0 },
    { pid: 640, name: "optimax.exe", cpu_percent: 0.4, memory_bytes: 0.2 * GIB, io_bytes_per_sec: 0 },
  ],
};
