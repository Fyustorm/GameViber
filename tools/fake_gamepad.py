#!/usr/bin/env python3
"""Fake "physical" Xbox 360 gamepad (uinput) for testing without hardware.

Prints the force feedback it receives (what GameViber forwards in proxy
passthrough mode) and can press A after a delay.

usage: fake_gamepad.py DURATION_S [PRESS_A_AFTER_S]
"""
import ctypes
import fcntl
import select
import sys
import time

import evdev
from evdev import AbsInfo, ff
from evdev import ecodes as e


def _ioc(direction, nr, struct):
    return (direction << 30) | (ctypes.sizeof(struct) << 16) | (ord("U") << 8) | nr


BEGIN_UPLOAD, END_UPLOAD = _ioc(3, 200, ff.UInputUpload), _ioc(1, 201, ff.UInputUpload)
BEGIN_ERASE, END_ERASE = _ioc(3, 202, ff.UInputErase), _ioc(1, 203, ff.UInputErase)

STICK = AbsInfo(0, -32768, 32767, 16, 128, 0)
CAPS = {
    e.EV_KEY: [e.BTN_SOUTH, e.BTN_EAST, e.BTN_NORTH, e.BTN_WEST, e.BTN_START, e.BTN_SELECT],
    e.EV_ABS: [(e.ABS_X, STICK), (e.ABS_Y, STICK)],
    e.EV_FF: [e.FF_RUMBLE, e.FF_PERIODIC, e.FF_SINE, e.FF_GAIN],
}


def main():
    duration = float(sys.argv[1]) if len(sys.argv) > 1 else 10.0
    press_at = time.time() + float(sys.argv[2]) if len(sys.argv) > 2 else None
    pad = evdev.UInput(CAPS, name="Microsoft X-Box 360 pad", vendor=0x045E, product=0x028E,
                       version=0x110, max_effects=16)
    print("fake gamepad:", pad.device.path, flush=True)
    end = time.time() + duration
    while time.time() < end:
        if press_at and time.time() > press_at:
            for value in (1, 0):
                pad.write(e.EV_KEY, e.BTN_SOUTH, value)
                pad.syn()
            press_at = None
            print("fake gamepad: pressed A", flush=True)
        if not select.select([pad.fd], [], [], 0.05)[0]:
            continue
        for ev in pad.read():
            if ev.type == e.EV_UINPUT and ev.code == e.UI_FF_UPLOAD:
                upload = ff.UInputUpload()
                upload.request_id = ev.value
                fcntl.ioctl(pad.fd, BEGIN_UPLOAD, upload)
                rumble = upload.effect.u.ff_rumble_effect
                print(f"fake gamepad: upload id={upload.effect.id} strong={rumble.strong_magnitude} "
                      f"weak={rumble.weak_magnitude}", flush=True)
                upload.retval = 0
                fcntl.ioctl(pad.fd, END_UPLOAD, upload)
            elif ev.type == e.EV_UINPUT and ev.code == e.UI_FF_ERASE:
                erase = ff.UInputErase()
                erase.request_id = ev.value
                fcntl.ioctl(pad.fd, BEGIN_ERASE, erase)
                print(f"fake gamepad: erase id={erase.effect_id}", flush=True)
                erase.retval = 0
                fcntl.ioctl(pad.fd, END_ERASE, erase)
            elif ev.type == e.EV_FF:
                print(f"fake gamepad: EV_FF code={ev.code} value={ev.value}", flush=True)
    pad.close()


if __name__ == "__main__":
    main()
