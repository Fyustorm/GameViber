# GameViber

Équivalent Linux de l'Intiface Game Haptics Router : intercepte le rumble
envoyé par les jeux à la manette et le relaie vers Intiface Central.

## Sources d'interception

| `--source` | Principe | Root | Le jeu voit |
|---|---|---|---|
| `proxy` (défaut) | Manette virtuelle uinput ; la vraie est grab, les inputs relayés, le rumble capturé puis renvoyé à la vraie manette | seulement pour `--hide` | une copie (même nom, VID/PID) — et la vraie si pas `--hide` |
| `ebpf` | Sonde eBPF sur les ioctl `EVIOCSFF`/`EVIOCRMFF` + lecture evdev passive des play/stop | oui | la vraie manette, rien ne change |

Les deux produisent les mêmes événements (effets, play/stop, gain, boutons,
axes), traités par le même moteur (`rumble.rs`, sémantique `ff-memless`).

La sonde eBPF est dérivée de
[linux-game-haptics-router](https://github.com/madrigal-eschat/linux-game-haptics-router)
(Apache-2.0, voir `LICENSE-APACHE-linux-game-haptics-router`), avec en plus la
capture du fd de l'ioctl pour savoir quelle manette est visée.

## Build

```sh
rustup toolchain install nightly --component rust-src   # pour la sonde eBPF
# bpf-linker : binaire précompilé sur https://github.com/aya-rs/bpf-linker/releases
cargo build --release          # SKIP_EBPF_BUILD=1 pour compiler sans la sonde
cargo test
```

## Utilisation

Démarrer Intiface Central (« Start Server ») puis, **avant le jeu** :

```sh
./target/release/gameviber                          # proxy
sudo ./target/release/gameviber --hide              # proxy, vraie manette masquée aux jeux
sudo ./target/release/gameviber --source ebpf       # observation passive
./target/release/gameviber --no-intiface -v         # juste logger rumble et boutons
```

Options : `--multiplier`, `--baseline`, `--combine avg|max`, `--no-passthrough`,
`--device /dev/input/eventX`, `--url ws://127.0.0.1:12345`, `--interval-ms`.

`tools/sdl_rumble.py` simule un jeu SDL3 (liste les manettes et fait vibrer chacune).

## Limites connues

- Seul le chemin evdev force-feedback est couvert : les manettes pilotées en
  hidraw par SDL (DualShock/DualSense/Switch) nécessitent `SDL_JOYSTICK_HIDAPI=0`.
- Source ebpf : les effets téléversés avant le lancement de GameViber sont
  invisibles jusqu'au prochain upload du jeu.

Le prototype Python d'origine est dans `prototype/`.
Spécification des modes scriptables : `docs/spec-modes.md`.
