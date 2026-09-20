#!/usr/bin/env python3
"""生成两帧示例 TM 二进制帧：frame_a.bin（正常）与 frame_b.bin（含故障）。"""

import struct
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "assets"


def primary_header(scid=0x2A, vcid=1, mc=0, vc=0, shf=1, fhp=0):
    w0 = (0 << 14) | (scid << 4) | (vcid << 1) | 0
    w1 = (shf << 15) | (0 << 14) | (0 << 13) | (0 << 11) | fhp
    return struct.pack(">HBBH", w0, mc, vc, w1)


def build_frame(mc, vc, ts_s, ts_sub, *, bus_v, bus_i, batt_t, soc, solar,
                payload_mode, heater, adcs, gyro_z, cpu, err, snr):
    frame = bytearray()
    frame += primary_header(mc=mc, vc=vc)
    frame += struct.pack(">IH", ts_s, ts_sub)          # 副帧头：时间戳
    d = bytearray()
    d += struct.pack(">H", round(bus_v / 0.1))          # bus_voltage
    d += struct.pack(">H", round(bus_i / 0.01))         # bus_current
    d += struct.pack(">h", round(batt_t / 0.1))         # batt_temp
    d += struct.pack(">B", soc)                         # batt_soc
    d += struct.pack(">H", solar)                       # solar_power
    # MSB 优先（CCSDS 约定）：payload_mode 占 bit0-1，heater 占 bit2，adcs 占 bit3-5
    packed = ((payload_mode & 0x3) << 6) | ((heater & 0x1) << 5) | ((adcs & 0x7) << 2)
    d += struct.pack(">B", packed)                      # payload_mode/heater/adcs
    d += struct.pack(">h", round(gyro_z / 0.01))        # gyro_z
    d += struct.pack(">B", cpu)                         # cpu_load
    d += struct.pack(">B", err)                         # obc_error_count
    d += struct.pack(">b", snr + 100)                   # downlink_snr (bias -100)
    d += struct.pack(">H", 0xBEEF)                      # frame_crc
    frame += d
    frame += bytes(32 - len(frame))                     # 填充至 32 字节
    assert len(frame) == 32
    return bytes(frame)


frame_a = build_frame(
    mc=10, vc=10, ts_s=1_758_000_000, ts_sub=0,
    bus_v=28.4, bus_i=3.2, batt_t=21.5, soc=88, solar=320,
    payload_mode=2, heater=0, adcs=2, gyro_z=0.05, cpu=34, err=0, snr=-72,
)

frame_b = build_frame(
    mc=11, vc=11, ts_s=1_758_000_004, ts_sub=32768,
    bus_v=22.8, bus_i=5.9, batt_t=47.2, soc=24, solar=38,
    payload_mode=3, heater=0, adcs=4, gyro_z=7.4, cpu=96, err=3, snr=-91,
)

(OUT / "frame_a.bin").write_bytes(frame_a)
(OUT / "frame_b.bin").write_bytes(frame_b)
print(f"wrote {OUT/'frame_a.bin'} and {OUT/'frame_b.bin'}")
