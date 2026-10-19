# GameViber

Feel your games on your toys. GameViber takes the rumble a game sends to your
gamepad, listens to its sound and looks at its image, and turns all of it into
vibrations for the toys connected to [Intiface Central](https://intiface.com/central/).
How it feels is decided by a **mode**: ideally one an AI assistant writes for
your game in a couple of minutes.

Linux (Steam, Proton, Lutris, Heroic: any game with a gamepad), and Windows
in preview, with fewer features (no in-game panel; see below).

**🌐 [gameviber.fyustorm.ovh](https://gameviber.fyustorm.ovh)**: what GameViber
does, the downloads, and the modes players made for their games.

[**⬇ Download**](https://gameviber.fyustorm.ovh/download) ·
[**🎮 Modes by game**](https://gameviber.fyustorm.ovh/games) ·
[User guide](docs/user-guide.md)

## Install

Download the file for your system from the
[website](https://gameviber.fyustorm.ovh/download) (it picks the one for your
system), or from the [Releases](https://github.com/Fyustorm/GameViber/releases) page:

| System | File | Install |
|---|---|---|
| Ubuntu 24.04+, Debian 13+, Mint 22+ | `.deb` | `sudo apt install ./gameviber_*.deb` |
| Fedora 40+ | `.rpm` | `sudo dnf install ./gameviber-*.rpm` |
| Arch, CachyOS, Manjaro | `.pkg.tar.zst` | `sudo pacman -U gameviber-*.pkg.tar.zst` |
| SteamOS, Bazzite, other systems | `.tar.gz` | extract it and run `./gameviber` (see its `README.txt`) |
| Windows 10 (2004+) and 11 | `-windows-x86_64-setup.exe` | run it: it also installs ViGEmBus and HidHide, which GameViber needs |
| Windows, without installing | `-windows-x86_64.zip` | extract it and run `gameviber.exe` (install [ViGEmBus](https://github.com/nefarius/ViGEmBus/releases) and [HidHide](https://github.com/nefarius/HidHide/releases) yourself) |

GameViber tells you when a new version is out and, with these files, installs
it for you (**Settings › Updates**).

You also need [Intiface Central](https://intiface.com/central/) to connect your
toys, and on Linux PipeWire for the game's sound (the default on recent systems).

On Windows, GameViber shows games a virtual Xbox 360 controller to hear their
rumble, hears the game's own sound, and reads the image of the game's window;
there is no in-game panel yet. It is new: tell us what does not work.

## First steps

1. Start Intiface Central and click **Start Server**.
2. Start **GameViber** (from your applications menu, or `gameviber`). Do not use
   `sudo`: GameViber asks for your password itself when it needs it.
3. Check that your toys show up in **Toys**, and your gamepad in **Setup**.
4. Get a mode made for your game: in **Library**, **Create a mode**, type
   your game's name, tick what you want to feel, then follow the Creator's
   **✨ AI assistant** tab: it proposes the game's phases, then writes the script.
   GameViber prepares a request to paste into any AI assistant (ChatGPT,
   Claude, Gemini...), and you paste its answer back.
5. Start your game and play.

If something feels wrong, open the mode's page and click **Doesn't feel
right?**: most answers come with a one-click fix.

## Modes

A mode decides how a game feels. Three ways to get one:

- **The community's**: the [modes players made and shared](https://gameviber.fyustorm.ovh/games),
  by game. Install them from the website (**Open in GameViber**) or from the
  app's **Community** page.
- **Your own**: an AI assistant writes one for your game (First steps above);
  share it from its page so others can play it.
- **The built-in ones**, for every genre while your game has none:

| Mode | For | Idea |
|---|---|---|
| Simple | anything | forwards the rumble as it is |
| Accumulation | anything | points earned per vibration, draining while idle |
| Combo | fighting (Tekken, SF6) | chained hits grow stronger, final burst |
| Overheat | action / shooter (Ratchet & Clank, Doom) | compressed rumble, peaks stand out, overheat gauge |
| Tension | turn-based with QTEs (Clair Obscur) | wave building up during fights, parries rewarded |
| Engine | racing (Forza, GT) | trigger = engine RPM, rumble = road and impacts |
| Heartbeat | horror (RE, Silent Hill) | heart speeding up with every vibration |
| All or Nothing | souls-like (Elden Ring, Sekiro) | gauge rising while you survive, emptied by a big hit |
| Ambient | exploration, platformer, cosy | slow wave under the rumble, fading out when idle |
| Surge | action metroidvania (PoP: The Lost Crown, Hollow Knight) | hits and parries fill a gauge, spent on a crescendo |

## Safety

- **STOP ALL** at the top of the window stops every toy at once.
- On the gamepad, hold **BACK + START** for half a second for the same panic
  stop (you can change the combo, or add a keyboard shortcut, in **Setup**).
- A global maximum intensity caps every mode.
- If the gamepad or Intiface disconnects, the toys stop.

## In-game overlay (optional)

A small panel over the game shows the mode, how strong the toys run and what
the mode detects. Turn it on per game in Steam's launch options:

- Proton (Windows) and Vulkan games: `GAMEVIBER_OVERLAY=1 %command%`
- Native OpenGL games: `gameviber-overlay %command%`

It runs inside the game, like MangoHud: **do not use it in online games with
an anti-cheat**. Everything else in GameViber works without it and never
touches the game.

## Privacy

Everything stays on your computer. The game's sound is never saved, and only
the screen captures you take yourself are kept. Nothing is sent to an AI
assistant unless you copy and paste it.

## License

GameViber is free software under the [GNU GPL version 3](LICENSE) or later:
use it, share it and improve it, as long as what you share stays free too.
The built-in modes, the mode API and the AI requests are under the
[MIT license](LICENSE-MIT): copy them freely into your own modes. Modes you
write are yours.

## Learn more

- [Website](https://gameviber.fyustorm.ovh): presentation, downloads, the
  community's modes.
- [User guide](docs/user-guide.md): every page of the app, a mode's inputs
  (phases, captures, indicators), the overlay, the built-in modes, known limitations.
- [Contributing](CONTRIBUTING.md): building, architecture, writing modes by
  hand, packaging.
