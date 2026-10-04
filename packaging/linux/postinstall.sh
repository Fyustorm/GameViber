#!/bin/sh
# Run by the package after installing or upgrading: applies the udev rule to
# /dev/uinput now, without a reboot. Polkit and the Vulkan loader read their
# files when needed.
udevadm control --reload-rules 2>/dev/null || true
udevadm trigger --sysname-match=uinput 2>/dev/null || true
