GameViber without a package
===========================

For systems where packages cannot be installed (SteamOS, Bazzite, Silverblue).
Prefer the .deb, .rpm or Arch package elsewhere: they also set up what is
described below, and update with the system.

Run ./gameviber from this directory (keep the libraries next to it). Then:

- In-game overlay: Setup › In-game overlay › Install. It is copied under
  ~/.local/share and updated when a newer GameViber starts.
- Proxy source (virtual gamepad): needs write access to /dev/uinput. Steam
  usually grants it already (its "steam-devices" udev rule). Otherwise, as root:
    echo 'KERNEL=="uinput", SUBSYSTEM=="misc", TAG+="uaccess", OPTIONS+="static_node=uinput"' \
      > /etc/udev/rules.d/60-gameviber-uinput.rules
    udevadm control --reload-rules && udevadm trigger --sysname-match=uinput
- eBPF source and hiding the real gamepad: GameViber asks for your password
  through pkexec when needed.
- The sound needs PipeWire's tools (pw-record, pw-dump, pw-link).
