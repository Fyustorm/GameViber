<script setup lang="ts">
import { computed } from 'vue'
import { api } from '@/api'
import { useLoad } from '@/load'
import { GITHUB, INTIFACE, TOYS, USER_GUIDE } from '@/links'
import { visitorSystem } from '@/system'
import GameCard from '@/components/GameCard.vue'

const games = useLoad(() => api.games())
const featured = computed(() => games.data.value?.slice(0, 8) ?? [])
const system = visitorSystem()
const download = system === 'windows' ? 'Download for Windows' : system === 'linux' ? 'Download for Linux' : 'Download'

// Each way in a few words: the guide says the rest.
const play = [
  'Modes for your game, the most played first',
  'Installed in a click, from this site or the app',
  'Tuned with a slider or two',
  'No mode for your game yet? Built-in ones fit every genre',
]

const tools = [
  'An AI assistant writes the script: no code',
  'Phases: exploration, battles, cutscenes',
  'Indicators: a health bar read from the screen',
  'Sessions replayed to tune without replaying',
  'Strokes and funscripts for strokers',
]

const steps = [
  {
    title: 'It reads the game',
    text: 'The rumble the game sends to your gamepad, with nothing to mod. Its sound. Its image: a health bar, a menu, a boss fight.',
  },
  {
    title: 'A mode decides the feel',
    text: 'A short script turns it into sensations: a slow wave in battles, a heartbeat when health runs low, a jolt on every hit.',
  },
  {
    title: 'Your toys play it',
    text: 'Through Intiface Central, so every toy it supports works. Several toys at once, each on its own channel.',
  },
]

const toys = [
  {
    kind: 'Vibrators',
    text: 'Every mode plays on them.',
    examples: 'Lovense Lush, Hush, Edge, Nora, Max · We-Vibe · Satisfyer Connect · Svakom · Magic Motion',
  },
  {
    kind: 'Strokers',
    text: 'Modes become strokes, faster and longer as the game heats up; some are written for them.',
    examples: 'The Handy · Kiiroo Keon · OSR2 and SR6 (to build yourself)',
  },
  {
    kind: 'Rotating and thrusting toys',
    text: 'They follow the intensity like vibrators do.',
    examples: "Lovense Nora's rotation · thrusting machines",
  },
]

const safety = [
  { title: 'STOP ALL', text: 'One button in the window stops every toy at once.' },
  { title: 'Panic combo', text: 'Hold BACK + START on the gamepad for the same stop, or set a keyboard shortcut.' },
  { title: 'A ceiling', text: 'A global maximum intensity caps every mode, whatever its script says.' },
  { title: 'Stops on loss', text: 'If the gamepad or Intiface disconnects, the toys stop.' },
]

const faq = [
  {
    question: 'Does it work on Windows?',
    answer:
      'Yes, on Windows 10 (2004) or later, with a few features less than on Linux: no panel inside the game, and the rumble goes through a virtual Xbox controller (ViGEmBus, which the installer offers).',
  },
  {
    question: 'Which games?',
    answer:
      'Any game you play with a gamepad: Steam, Epic, GOG, Proton, Lutris, Heroic... The rumble is the main signal; the sound and the image add the rest, even in games that barely vibrate.',
  },
  {
    question: 'Which gamepads?',
    answer:
      'Xbox, PlayStation, Switch Pro, 8BitDo and most others. A gamepad your system does not know gets its buttons set up once, one press at a time.',
  },
  {
    question: 'Which toys?',
    answer:
      'Every toy Intiface Central supports: vibrators, strokers, rotating toys, from Lovense, We-Vibe, Kiiroo, The Handy and many more.',
  },
  {
    question: 'Can an anti-cheat ban me?',
    answer:
      'GameViber reads the gamepad, the sound and the image from outside the game. Only the in-game overlay on Linux runs inside the game, like MangoHud: do not turn it on in online games with an anti-cheat. Modes reading the screen need it on Linux; on Windows nothing is loaded into the game.',
  },
  {
    question: 'Do I need to code?',
    answer:
      'No. Play the modes others made, or have an AI assistant write yours: GameViber writes the request, you paste the answer. Coders can edit the Luau script directly.',
  },
  {
    question: 'What about my data?',
    answer:
      'Everything stays on your computer: the sound is never saved, and only the screenshots you take are kept. Nothing goes to an AI assistant unless you paste it. Play time of community modes is shared only if you agree at first launch, under a random id.',
  },
  {
    question: 'Does it cost anything?',
    answer: 'No. GameViber is free and open source (GPL-3.0), with no account to play. An account is only needed to publish your modes.',
  },
]
</script>

<template>
  <section class="hero">
    <div class="wrap hero-inner">
      <div class="pitch">
        <p class="eyebrow">Free · Open source · Windows &amp; Linux</p>
        <h1>Feel your games on your toys.</h1>
        <p class="lead">
          GameViber takes the rumble a game sends to your gamepad, listens to its sound and looks at its image, and
          turns all of it into sensations on your vibrators, strokers and every toy
          <a target="_blank" rel="noopener" :href="INTIFACE">Intiface Central</a> connects.
        </p>
        <div class="actions">
          <RouterLink to="/download" class="button primary">{{ download }}</RouterLink>
          <RouterLink to="/games" class="button">Browse community modes</RouterLink>
        </div>
        <p class="muted small">Steam, Epic, GOG, Proton, Lutris, Heroic: any game played with a gamepad.</p>
      </div>
      <figure class="hero-shot">
        <img
          src="/screenshots/overlay.webp"
          width="1600"
          height="900"
          alt="A game with GameViber's overlay in a corner: the mode, the output sent to the toys, health and a gauge read from the screen."
          class="shot"
          fetchpriority="high"
        />
        <figcaption class="muted">The in-game overlay on Linux: the mode, what it sends, what it reads from the screen.</figcaption>
      </figure>
    </div>
  </section>

  <section class="section">
    <div class="wrap">
      <p class="eyebrow">Two ways to play</p>
      <h2>Pick a mode, or make your own</h2>
      <div class="grid two ways">
        <div class="card way">
          <p class="eyebrow">Ready in a click</p>
          <h3>Play a mode the community made</h3>
          <ul class="points">
            <li v-for="point in play" :key="point">{{ point }}</li>
          </ul>
          <RouterLink to="/games" class="button primary">Browse community modes</RouterLink>
        </div>
        <div class="card way">
          <p class="eyebrow">A workshop for your game</p>
          <h3>Make your own mode</h3>
          <ul class="points">
            <li v-for="point in tools" :key="point">{{ point }}</li>
          </ul>
          <a target="_blank" rel="noopener" :href="USER_GUIDE" class="button">Read the guide</a>
        </div>
      </div>
    </div>
  </section>

  <section class="section alt">
    <div class="wrap">
      <p class="eyebrow">How it works</p>
      <h2>From the game to the toy</h2>
      <ol class="grid three steps">
        <li v-for="(step, i) in steps" :key="step.title" class="card">
          <span class="number" aria-hidden="true">{{ i + 1 }}</span>
          <h3>{{ step.title }}</h3>
          <p class="muted">{{ step.text }}</p>
        </li>
      </ol>
    </div>
  </section>

  <section class="section">
    <div class="wrap feature">
      <div>
        <p class="eyebrow">Read the screen</p>
        <h2>Draw it once, GameViber reads it</h2>
        <p class="muted">
          Capture the game, draw a box over a health bar or a menu: GameViber reads it while you play, as a gauge or
          as shown and hidden. Your captures also teach it the game's phases.
        </p>
        <p class="muted">An AI assistant can propose the phases and the indicators worth drawing for your game.</p>
      </div>
      <img
        src="/screenshots/captures.webp"
        width="1600"
        height="859"
        alt="The Captures and indicators tab: screenshots of the game sorted by phase, and a gauge drawn over its health bar."
        class="shot"
        loading="lazy"
      />
    </div>
  </section>

  <section class="section alt">
    <div class="wrap feature reverse">
      <div>
        <p class="eyebrow">Tune it</p>
        <h2>Replay a session, feel it again</h2>
        <p class="muted">
          Record a play session, then replay it into the mode and watch it like a video while the toys play it. Change a
          setting, feel the difference, without playing the same fight again.
        </p>
        <p class="muted">
          Something feels wrong? <strong>Doesn't feel right?</strong> asks what, and most answers come with a one-click
          fix.
        </p>
      </div>
      <img
        src="/screenshots/replay.webp"
        width="1600"
        height="867"
        alt="The session player: a recorded game replayed with its timeline, what the mode read and what it sent to the toys."
        class="shot"
        loading="lazy"
      />
    </div>
  </section>

  <section class="section">
    <div class="wrap">
      <p class="eyebrow">Toys</p>
      <h2>Every toy Intiface supports</h2>
      <p class="muted intro">
        GameViber drives your toys through <a target="_blank" rel="noopener" :href="INTIFACE">Intiface Central</a>, which speaks to hundreds of them
        over Bluetooth or USB. Several toys at once, each on its own channel.
      </p>
      <div class="grid three">
        <div v-for="toy in toys" :key="toy.kind" class="card">
          <h3>{{ toy.kind }}</h3>
          <p class="muted">{{ toy.text }}</p>
          <p class="examples">{{ toy.examples }}</p>
        </div>
      </div>
      <p class="more"><a target="_blank" rel="noopener" :href="TOYS">See every supported toy ›</a></p>
    </div>
  </section>

  <section class="section alt">
    <div class="wrap">
      <div class="heading-row">
        <div>
          <p class="eyebrow">Community</p>
          <h2>Modes other players made</h2>
        </div>
        <RouterLink to="/games" class="button small">All games</RouterLink>
      </div>
      <div v-if="games.loading.value" class="grid four">
        <div v-for="n in 4" :key="n" class="skeleton tile" />
      </div>
      <p v-else-if="games.error.value" class="state error">The community modes cannot be reached right now.</p>
      <p v-else-if="!featured.length" class="state">No mode published yet: yours could be the first, from the app's Sharing tab.</p>
      <div v-else class="grid four">
        <GameCard v-for="game in featured" :key="game.id" :game="game" />
      </div>
    </div>
  </section>

  <section class="section">
    <div class="wrap">
      <p class="eyebrow">Safety</p>
      <h2>You stay in control</h2>
      <div class="grid four">
        <div v-for="item in safety" :key="item.title" class="card">
          <h3>{{ item.title }}</h3>
          <p class="muted">{{ item.text }}</p>
        </div>
      </div>
    </div>
  </section>

  <section class="section alt">
    <div class="wrap faq">
      <p class="eyebrow">Questions</p>
      <h2>Good to know</h2>
      <details v-for="item in faq" :key="item.question" class="card">
        <summary>{{ item.question }}</summary>
        <p class="muted">{{ item.answer }}</p>
      </details>
    </div>
  </section>

  <section class="section">
    <div class="wrap closing">
      <h2>Ready to play?</h2>
      <p class="muted">
        Free and open source (GPL-3.0), for Windows and Linux. Read the code, report a bug or help on
        <a target="_blank" rel="noopener" :href="GITHUB">GitHub</a>.
      </p>
      <div class="actions">
        <RouterLink to="/download" class="button primary">{{ download }}</RouterLink>
        <a target="_blank" rel="noopener" :href="GITHUB" class="button">GitHub</a>
      </div>
    </div>
  </section>
</template>

<style scoped>
.hero {
  padding: 32px 0 8px;
  background: radial-gradient(120% 80% at 80% 0%, rgb(240 109 148 / 0.12), transparent 60%);
}

.hero-inner {
  display: grid;
  gap: 28px;
}

.pitch .actions {
  margin: 20px 0 12px;
}

.small {
  font-size: 14px;
}

.hero-shot {
  margin: 0;
}

figcaption {
  font-size: 13px;
  margin-top: 8px;
}

.alt {
  background: #15171d;
  border-block: 1px solid var(--line);
}

.steps {
  list-style: none;
  padding: 0;
  margin: 20px 0 0;
}

.number {
  display: inline-grid;
  place-items: center;
  width: 32px;
  height: 32px;
  border-radius: 50%;
  background: var(--accent-soft);
  color: var(--accent-text);
  font-weight: 700;
  margin-bottom: 12px;
}

.way {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  padding: 20px;
}

.way .button {
  margin-top: auto;
}

.points {
  list-style: none;
  padding: 0;
  margin: 4px 0 20px;
  display: grid;
  gap: 8px;
  color: var(--muted);
}

.points li {
  padding-left: 22px;
  position: relative;
}

.points li::before {
  content: '';
  position: absolute;
  left: 4px;
  top: 0.6em;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--accent);
}

.intro {
  max-width: 720px;
}

.examples {
  font-size: 14px;
  margin: 0;
}

.more {
  margin: 16px 0 0;
}

.faq {
  max-width: 800px;
}

.faq details {
  margin-bottom: 10px;
}

.faq summary {
  cursor: pointer;
  font-weight: 600;
  min-height: 28px;
}

.faq details p {
  margin: 10px 0 0;
}

.feature {
  display: grid;
  gap: 24px;
  align-items: center;
}

.heading-row {
  display: flex;
  justify-content: space-between;
  align-items: flex-end;
  gap: 16px;
  margin-bottom: 20px;
}

.heading-row h2 {
  margin: 0;
}

.tile {
  aspect-ratio: 460 / 260;
}

.closing {
  text-align: center;
  max-width: 640px;
}

.closing .actions {
  justify-content: center;
}

.grid.four .card h3 {
  font-size: 16px;
}

@media (min-width: 640px) {
  .hero {
    padding-top: 56px;
  }
}

@media (min-width: 900px) {
  .hero-inner {
    grid-template-columns: 5fr 6fr;
    align-items: center;
    gap: 40px;
  }

  .feature {
    grid-template-columns: 2fr 3fr;
    gap: 48px;
  }

  .feature.reverse > div {
    order: 2;
  }
}
</style>
