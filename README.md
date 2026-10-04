# GameViber

Feel your games on your toys. GameViber takes the rumble a game sends to your
gamepad, listens to its sound and looks at its image, and turns all of it into
vibrations for the toys connected to [Intiface Central](https://intiface.com/central/).
How it feels is decided by a **mode**: ideally one an AI assistant writes for
your game in a couple of minutes.

Linux only for now (Steam, Proton, Lutris, Heroic: any game with a gamepad).

## Install

Download the package for your system from the
[Releases](https://github.com/Fyustorm/GameViber/releases) page:

| System | File | Install |
|---|---|---|
| Ubuntu 24.04+, Debian 13+, Mint 22+ | `.deb` | `sudo apt install ./gameviber_*.deb` |
| Fedora 40+ | `.rpm` | `sudo dnf install ./gameviber-*.rpm` |
| Arch, CachyOS, Manjaro | `.pkg.tar.zst` | `sudo pacman -U gameviber-*.pkg.tar.zst` |
| SteamOS, Bazzite, other systems | `.tar.gz` | extract it and run `./gameviber` (see its `README.txt`) |

GameViber tells you when a new version is out and, with these files, installs
it for you (**Settings › Updates**).

You also need [Intiface Central](https://intiface.com/central/) to connect your
toys, and PipeWire for the game's sound (the default on recent systems).

## First steps

1. Start Intiface Central and click **Start Server**.
2. Start **GameViber** (from your applications menu, or `gameviber`). Do not use
   `sudo`: GameViber asks for your password itself when it needs it.
3. Follow the setup guide: your toys, your gamepad, then a mode.
4. Get a mode made for your game: in **Games**, add your game, then
   **New mode**. GameViber prepares a request to paste into any AI assistant
   (ChatGPT, Claude, Gemini, Le Chat...), and you paste its answer back.
5. Start your game and play.

If something feels wrong, open the mode's page and click **Doesn't feel
right?**: most answers come with a one-click fix.

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

- [User guide](docs/user-guide.md): every page of the app, a game's signals
  (scenes, captures, zones), the overlay, the built-in modes, known limitations.
- [Contributing](CONTRIBUTING.md): building, architecture, writing modes by
  hand, packaging.
