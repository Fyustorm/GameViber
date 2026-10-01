# GameViber

Équivalent Linux de l'Intiface Game Haptics Router : intercepte le rumble
envoyé par les jeux à la manette, le transforme via un **mode scriptable en
Lua** et pilote les jouets connectés à Intiface Central.

## Utilisation

Démarrer Intiface Central (« Start Server ») puis, **avant le jeu** :

```sh
./target/release/gameviber                      # GUI
./target/release/gameviber --headless -v        # sans GUI, logs seulement
```

Ne pas lancer GameViber avec `sudo` : la source eBPF et le masquage de la
manette passent par un **helper privilégié** démarré via `pkexec` à la
demande (une seule demande de mot de passe par session).

La GUI propose :

- une barre d'état avec **STOP ALL** (aussi : BACK + START maintenus 0,5 s sur
  la manette), l'intensité maximale globale, le **choix de la source**
  (proxy / eBPF / aucune) et la case « masquer la vraie manette » ;
- la liste des modes et leurs paramètres, générés à partir du script ;
- **Monitor** : graphes du rumble, des sorties et des valeurs `plot()` ;
- **Editor** : édition des modes avec rechargement à chaud (Ctrl+S) ;
- **Routing** : quels jouets chaque canal du mode pilote ;
- **Simulator** : faux rumble et faux boutons pour tester sans jeu ;
- **Log**.

Les modes utilisateur sont des fichiers `.luau` dans `~/.config/gameviber/modes/`
(modifiables aussi dans un éditeur externe : ils sont rechargés à la sauvegarde).
API : [`docs/spec-modes.md`](docs/spec-modes.md). Modes fournis :
[`gameviber/modes/`](gameviber/modes/).

Options : `--source proxy|ebpf|none` (mémorisée ensuite), `--device /dev/input/eventX`,
`--hide`, `--no-passthrough`, `--url`, `--no-intiface`, `--mode <fichier ou nom>`, `-v`.

## Sources d'interception

| `--source` | Principe | Root | Le jeu voit |
|---|---|---|---|
| `proxy` (défaut) | Manette virtuelle uinput ; la vraie est grab, les inputs relayés, le rumble capturé puis renvoyé à la vraie manette | pour masquer la vraie (helper) | une copie (même nom, VID/PID) et la vraie si elle n'est pas masquée |
| `ebpf` | Sonde eBPF sur les ioctl `EVIOCSFF`/`EVIOCRMFF` + lecture evdev passive des play/stop | oui (helper) | la vraie manette, rien ne change |
| `none` | Aucune interception (simulateur seulement) | non | — |

La sonde eBPF est dérivée de
[linux-game-haptics-router](https://github.com/madrigal-eschat/linux-game-haptics-router)
(Apache-2.0, voir `LICENSE-APACHE-linux-game-haptics-router`), avec en plus la
capture du fd de l'ioctl pour savoir quelle manette est visée.

### Helper privilégié

`gameviber helper` est le seul code exécuté en root. Il ne fait que charger
la sonde eBPF (et transmettre ses événements bruts) et masquer / restaurer les
nœuds d'une manette (`/dev/input/eventN` uniquement). Il dialogue en JSON par
stdin/stdout avec le processus utilisateur, ignore Ctrl+C, et restaure tout
puis s'arrête dès que stdin se ferme (fermeture ou crash de GameViber). La
résolution des fd, la lecture des manettes et tout le reste tournent sans
privilège. Lancé directement en root (`sudo ... --headless`), GameViber se
passe du helper.

## Build

```sh
rustup toolchain install nightly --component rust-src   # pour la sonde eBPF
# bpf-linker : binaire précompilé sur https://github.com/aya-rs/bpf-linker/releases
cargo build --release          # SKIP_EBPF_BUILD=1 pour compiler sans la sonde
cargo test
```

Luau est compilé depuis les sources (compilateur C++ requis).

## Limites connues

- Seul le chemin evdev force-feedback est couvert : les manettes pilotées en
  hidraw par SDL (DualShock/DualSense/Switch) nécessitent `SDL_JOYSTICK_HIDAPI=0`.
- Source ebpf : les effets téléversés avant le lancement de GameViber sont
  invisibles jusqu'au prochain upload du jeu.

`tools/sdl_rumble.py` simule un jeu SDL3, `tools/fake_gamepad.py` une manette physique. Le prototype Python d'origine est dans `prototype/`.
