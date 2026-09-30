#!/usr/bin/env python3
"""GameViber - prototype (approche A : manette virtuelle uinput).

La vraie manette est "grab" (le jeu ne reçoit plus ses événements) et une
manette virtuelle avec le même nom / VID / PID est créée. Les inputs sont
relayés réelle -> virtuelle ; le force-feedback téléversé par le jeu sur la
virtuelle est relayé vers la vraie manette (passthrough) et converti en
commandes Buttplug envoyées à Intiface Central.
"""
import argparse
import asyncio
import ctypes
import fcntl
import json
import logging
import os
import subprocess
import time

import evdev
import websockets
from evdev import ecodes as e, ff

log = logging.getLogger("gameviber")

PHYS_TAG = "gameviber"
FF_CODES = [e.FF_RUMBLE, e.FF_PERIODIC, e.FF_SQUARE, e.FF_TRIANGLE, e.FF_SINE, e.FF_GAIN]


# --- ioctl uinput FF (python-evdev ne renseigne pas request_id dans begin_upload/begin_erase)

def _ioc(direction, nr, struct):
    return (direction << 30) | (ctypes.sizeof(struct) << 16) | (ord("U") << 8) | nr

UI_BEGIN_FF_UPLOAD = _ioc(3, 200, ff.UInputUpload)
UI_END_FF_UPLOAD = _ioc(1, 201, ff.UInputUpload)
UI_BEGIN_FF_ERASE = _ioc(3, 202, ff.UInputErase)
UI_END_FF_ERASE = _ioc(1, 203, ff.UInputErase)


def effect_motors(effect):
    """(strong, weak) en 0..65535 pour un effet, périodiques approximés comme ff-memless."""
    if effect.type == e.FF_RUMBLE:
        r = effect.u.ff_rumble_effect
        return r.strong_magnitude, r.weak_magnitude
    if effect.type == e.FF_PERIODIC:
        m = min(abs(effect.u.ff_periodic_effect.magnitude) * 2, 0xFFFF)
        return m, m
    return 0, 0


def find_gamepad(path):
    if path:
        return evdev.InputDevice(path)
    for p in evdev.list_devices():
        dev = evdev.InputDevice(p)
        caps = dev.capabilities()
        if (not (dev.phys or "").startswith(PHYS_TAG)
                and e.FF_RUMBLE in caps.get(e.EV_FF, [])
                and e.BTN_SOUTH in caps.get(e.EV_KEY, [])):
            return dev
        dev.close()
    raise SystemExit("Aucune manette avec rumble trouvée")


class RumbleState:
    """Rejoue la sémantique FF evdev (upload / play / stop / gain) pour connaître l'état des moteurs."""

    def __init__(self):
        self.effects = {}   # id virtuel -> Effect
        self.active = {}    # id virtuel -> (début, fin ou None)
        self.gain = 0xFFFF

    def play(self, eid, count):
        effect = self.effects.get(eid)
        if effect is None or count == 0:
            self.active.pop(eid, None)
            return
        start = time.monotonic() + effect.ff_replay.delay / 1000
        length = effect.ff_replay.length / 1000
        self.active[eid] = (start, start + length * count if length else None)

    def erase(self, eid):
        self.effects.pop(eid, None)
        self.active.pop(eid, None)

    def motors(self):
        now = time.monotonic()
        strong = weak = 0
        for eid, (start, end) in list(self.active.items()):
            if end is not None and now >= end:
                del self.active[eid]
                continue
            if now < start or eid not in self.effects:
                continue
            s, w = effect_motors(self.effects[eid])
            strong, weak = max(strong, s), max(weak, w)
        g = self.gain / 0xFFFF
        return int(strong * g), int(weak * g)


class DeviceHider:
    """Rend la vraie manette invisible aux jeux (nécessite root) : chmod 0600 et
    suppression des ACL uaccess sur ses nœuds eventX / jsX, restaurés à la sortie."""

    def __init__(self, real):
        sysdir = f"/sys/class/input/{os.path.basename(real.path)}/device"
        self.nodes = [f"/dev/input/{n}" for n in os.listdir(sysdir) if n.startswith(("event", "js"))]
        self.saved = {}

    def hide(self):
        for node in self.nodes:
            acl = subprocess.run(["getfacl", "-p", node], capture_output=True, text=True, check=True).stdout
            self.saved[node] = (os.stat(node).st_mode & 0o7777, acl)
            subprocess.run(["setfacl", "-b", node], check=True)
            os.chmod(node, 0o600)
            log.info("Manette réelle masquée: %s", node)

    def restore(self):
        for node, (mode, acl) in self.saved.items():
            try:
                os.chmod(node, mode)
                subprocess.run(["setfacl", "--restore=-"], input=acl, text=True, check=True)
            except (OSError, subprocess.CalledProcessError) as err:
                log.warning("Restauration de %s échouée: %s", node, err)


class GamepadProxy:
    def __init__(self, real, passthrough):
        self.real = real
        self.passthrough = passthrough
        self.state = RumbleState()
        self.real_ids = {}  # id virtuel -> id sur la vraie manette

        caps = real.capabilities(absinfo=True)
        caps.pop(e.EV_SYN, None)
        caps[e.EV_FF] = FF_CODES
        self.ui = evdev.UInput(
            events=caps,
            name=real.name,
            vendor=real.info.vendor,
            product=real.info.product,
            version=real.info.version,
            bustype=real.info.bustype,
            phys=f"{PHYS_TAG}/{real.phys}",
            max_effects=real.ff_effects_count or 16,
        )
        real.grab()
        log.info("Manette virtuelle '%s' créée sur %s (réelle %s grab)", real.name, self.ui.device.path, real.path)

    def close(self):
        for rid in self.real_ids.values():
            try:
                self.real.erase_effect(rid)
            except OSError:
                pass
        try:
            self.real.ungrab()
        except OSError:
            pass
        self.ui.close()

    # --- réelle -> virtuelle
    def on_real_readable(self):
        for ev in self.real.read():
            if ev.type != e.EV_FF:
                self.ui.write(ev.type, ev.code, ev.value)

    # --- jeu -> virtuelle (FF)
    def on_virtual_readable(self):
        for ev in self.ui.read():
            if ev.type == e.EV_UINPUT:
                if ev.code == e.UI_FF_UPLOAD:
                    self._upload(ev.value)
                elif ev.code == e.UI_FF_ERASE:
                    self._erase(ev.value)
            elif ev.type == e.EV_FF:
                self._ff_event(ev.code, ev.value)

    def _upload(self, request_id):
        up = ff.UInputUpload()
        up.request_id = request_id
        fcntl.ioctl(self.ui.fd, UI_BEGIN_FF_UPLOAD, up)
        effect = ff.Effect.from_buffer_copy(up.effect)
        vid = effect.id
        self.state.effects[vid] = effect
        up.retval = 0
        if self.passthrough:
            real_effect = ff.Effect.from_buffer_copy(effect)
            real_effect.id = self.real_ids.get(vid, -1)
            try:
                self.real_ids[vid] = self.real.upload_effect(real_effect)
            except OSError as err:
                log.warning("Upload sur la vraie manette échoué: %s", err)
        fcntl.ioctl(self.ui.fd, UI_END_FF_UPLOAD, up)
        s, w = effect_motors(effect)
        log.debug("upload id=%d type=%#x strong=%d weak=%d len=%dms", vid, effect.type, s, w, effect.ff_replay.length)

    def _erase(self, request_id):
        er = ff.UInputErase()
        er.request_id = request_id
        fcntl.ioctl(self.ui.fd, UI_BEGIN_FF_ERASE, er)
        self.state.erase(er.effect_id)
        rid = self.real_ids.pop(er.effect_id, None)
        if rid is not None:
            try:
                self.real.erase_effect(rid)
            except OSError:
                pass
        er.retval = 0
        fcntl.ioctl(self.ui.fd, UI_END_FF_ERASE, er)

    def _ff_event(self, code, value):
        if code == e.FF_GAIN:
            self.state.gain = value
            if self.passthrough:
                self.real.write(e.EV_FF, e.FF_GAIN, value)
            return
        self.state.play(code, value)
        rid = self.real_ids.get(code)
        if self.passthrough and rid is not None:
            self.real.write(e.EV_FF, rid, value)
        log.debug("play id=%d count=%d", code, value)


class IntifaceClient:
    """Client Buttplug minimal (protocole v3, JSON sur websocket)."""

    def __init__(self, url):
        self.url = url
        self.ws = None
        self.server = None
        self.devices = {}  # index -> DeviceMessages
        self._msg_id = 0

    def _next_id(self):
        self._msg_id += 1
        return self._msg_id

    async def _send(self, name, **fields):
        await self.ws.send(json.dumps([{name: {"Id": self._next_id(), **fields}}]))

    async def run(self):
        while True:
            try:
                async with websockets.connect(self.url) as ws:
                    self.ws = ws
                    # Handshake : rien d'autre ne doit partir avant la réponse ServerInfo.
                    await self._send("RequestServerInfo", ClientName="GameViber", MessageVersion=3)
                    for msg in json.loads(await ws.recv()):
                        self._handle(msg)
                    if self.server is None:
                        raise websockets.WebSocketException("handshake refusé")
                    await self._send("RequestDeviceList")
                    await self._send("StartScanning")
                    log.info("Connecté à Intiface '%s' (%s)", self.server, self.url)
                    async for raw in ws:
                        for msg in json.loads(raw):
                            self._handle(msg)
            except (OSError, websockets.WebSocketException) as err:
                log.warning("Intiface indisponible (%s), nouvelle tentative dans 5 s", err)
            self.ws = None
            self.server = None
            self.devices.clear()
            await asyncio.sleep(5)

    def _handle(self, msg):
        (name, body), = msg.items()
        if name == "ServerInfo":
            self.server = body.get("ServerName", "?")
            if body.get("MaxPingTime"):
                asyncio.create_task(self._ping_loop(body["MaxPingTime"] / 2000))
        elif name == "DeviceList":
            for dev in body["Devices"]:
                self._add(dev)
        elif name == "DeviceAdded":
            self._add(body)
        elif name == "DeviceRemoved":
            self.devices.pop(body["DeviceIndex"], None)
            log.info("Appareil retiré: %d", body["DeviceIndex"])
        elif name == "Error":
            log.warning("Erreur Intiface: %s", body.get("ErrorMessage"))

    def _add(self, dev):
        self.devices[dev["DeviceIndex"]] = dev["DeviceMessages"]
        log.info("Appareil: [%d] %s", dev["DeviceIndex"], dev["DeviceName"])

    async def _ping_loop(self, period):
        while self.ws is not None:
            await self._send("Ping")
            await asyncio.sleep(period)

    async def set_speed(self, speed):
        if self.ws is None:
            return
        for index, messages in self.devices.items():
            scalars = [
                {"Index": i, "Scalar": speed, "ActuatorType": a["ActuatorType"]}
                for i, a in enumerate(messages.get("ScalarCmd", []))
                if a["ActuatorType"] in ("Vibrate", "Oscillate")
            ]
            if scalars:
                await self._send("ScalarCmd", DeviceIndex=index, Scalars=scalars)
            rotations = [{"Index": i, "Speed": speed, "Clockwise": True}
                         for i in range(len(messages.get("RotateCmd", [])))]
            if rotations:
                await self._send("RotateCmd", DeviceIndex=index, Rotations=rotations)

    async def stop(self):
        if self.ws is not None:
            await self._send("StopAllDevices")


def to_speed(strong, weak, args):
    """Même logique que GHR (MainWindow.xaml.cs) : moyenne (ou max) x multiplicateur, plancher baseline."""
    level = max(strong, weak) if args.mode == "max" else (strong + weak) / 2
    if level == 0 and args.baseline == 0:
        return 0.0
    return min(max(level / 0xFFFF * args.multiplier, args.baseline), 1.0)


async def main(args):
    proxy = GamepadProxy(find_gamepad(args.device), passthrough=not args.no_passthrough)
    hider = None
    if args.hide:
        hider = DeviceHider(proxy.real)
        hider.hide()
    intiface = None if args.no_intiface else IntifaceClient(args.url)
    loop = asyncio.get_running_loop()
    loop.add_reader(proxy.real.fd, proxy.on_real_readable)
    loop.add_reader(proxy.ui.fd, proxy.on_virtual_readable)
    tasks = [asyncio.create_task(intiface.run())] if intiface else []

    last = None
    try:
        while True:
            strong, weak = proxy.state.motors()
            speed = round(to_speed(strong, weak, args), 2)
            if speed != last:
                log.info("rumble strong=%5d weak=%5d -> vitesse %.2f", strong, weak, speed)
                if intiface:
                    await intiface.set_speed(speed)
                last = speed
            await asyncio.sleep(args.interval)
    finally:
        loop.remove_reader(proxy.real.fd)
        loop.remove_reader(proxy.ui.fd)
        if intiface:
            await intiface.stop()
        for t in tasks:
            t.cancel()
        if hider:
            hider.restore()
        proxy.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--device", help="chemin /dev/input/eventX de la manette (auto sinon)")
    parser.add_argument("--url", default="ws://127.0.0.1:12345", help="URL du serveur Intiface")
    parser.add_argument("--no-intiface", action="store_true", help="ne fait que logger le rumble")
    parser.add_argument("--no-passthrough", action="store_true", help="ne pas faire vibrer la vraie manette")
    parser.add_argument("--hide", action="store_true",
                        help="masquer la vraie manette aux jeux pendant l'exécution (root requis)")
    parser.add_argument("--multiplier", type=float, default=1.0)
    parser.add_argument("--baseline", type=float, default=0.0, help="vitesse minimale 0..1")
    parser.add_argument("--mode", choices=["avg", "max"], default="avg", help="combinaison des 2 moteurs")
    parser.add_argument("--interval", type=float, default=0.05, help="période d'envoi vers Intiface (s)")
    parser.add_argument("-v", "--verbose", action="store_true")
    args = parser.parse_args()
    if args.hide and os.geteuid() != 0:
        parser.error("--hide nécessite root (sudo)")
    logging.basicConfig(level=logging.DEBUG if args.verbose else logging.INFO,
                        format="%(asctime)s %(levelname)-7s %(message)s", datefmt="%H:%M:%S")
    logging.getLogger("websockets").setLevel(logging.INFO)
    logging.getLogger("asyncio").setLevel(logging.INFO)
    try:
        asyncio.run(main(args))
    except KeyboardInterrupt:
        pass
