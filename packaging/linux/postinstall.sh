#!/bin/sh
# Run by the package after installing or upgrading: applies the udev rule to
# /dev/uinput now, without a reboot, and tells the desktop GameViber opens
# gameviber:// links and about its icon. Polkit and the Vulkan loader read their files when needed.
udevadm control --reload-rules 2>/dev/null || true
udevadm trigger --sysname-match=uinput 2>/dev/null || true
update-desktop-database -q /usr/share/applications 2>/dev/null || true
gtk-update-icon-cache -q -t /usr/share/icons/hicolor 2>/dev/null || true
