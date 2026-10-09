<script setup lang="ts">
import { computed } from 'vue'
import { api } from '@/api'
import { useLoad } from '@/load'
import { GITHUB, INTIFACE } from '@/links'
import GameCard from '@/components/GameCard.vue'

const games = useLoad(() => api.games())
const featured = computed(() => games.data.value?.slice(0, 8) ?? [])

const steps = [
  {
    title: 'It reads the game',
    text: 'The rumble the game sends to your gamepad, with nothing to mod. Its sound. Its image, through a small in-game overlay: a health bar, a menu, a boss fight.',
  },
  {
    title: 'A mode decides the feel',
    text: 'A short script turns it into vibrations: a slow wave in battles, a heartbeat when health runs low, a jolt on every hit. Each game can have its own.',
  },
  {
    title: 'Your toys play it',
    text: 'Through Intiface Central, so every toy it supports works. Several toys at once, each on its own channel.',
  },
]

const safety = [
  { title: 'STOP ALL', text: 'One button in the window stops every toy at once.' },
  { title: 'Panic combo', text: 'Hold BACK + START on the gamepad for the same stop, or set a keyboard shortcut.' },
  { title: 'A ceiling', text: 'A global maximum intensity caps every mode, whatever its script says.' },
  { title: 'Stops on loss', text: 'If the gamepad or Intiface disconnects, the toys stop.' },
]
</script>

<template>
  <section class="hero">
    <div class="wrap hero-inner">
      <div class="pitch">
        <p class="eyebrow">Free · Open source · Linux</p>
        <h1>Feel your games on your toys.</h1>
        <p class="lead">
          GameViber takes the rumble a game sends to your gamepad, listens to its sound and looks at its image, and
          turns all of it into vibrations for the toys connected to
          <a :href="INTIFACE">Intiface Central</a>.
        </p>
        <div class="actions">
          <RouterLink to="/download" class="button primary">Download for Linux</RouterLink>
          <RouterLink to="/games" class="button">Browse community modes</RouterLink>
        </div>
        <p class="muted small">Steam, Proton, Lutris, Heroic: any game played with a gamepad.</p>
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
        <figcaption class="muted">The in-game overlay: the mode, what it sends, what it reads from the screen.</figcaption>
      </figure>
    </div>
  </section>

  <section class="section">
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

  <section class="section alt">
    <div class="wrap feature">
      <div>
        <p class="eyebrow">A mode for your game</p>
        <h2>Written for you in a couple of minutes</h2>
        <p class="muted">
          Built-in modes work with any game, but the best ones know yours. GameViber writes the request: paste it into
          any AI assistant (ChatGPT, Claude, Gemini, Le Chat...), paste its answer back, play. No code to write.
        </p>
        <p class="muted">
          Show it what the game looks like: draw a health bar or a menu on a screenshot once, and GameViber reads it
          while you play.
        </p>
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

  <section class="section">
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
    <div class="wrap closing">
      <h2>Ready to play?</h2>
      <p class="muted">
        Free and open source (GPL-3.0). Linux today; Windows is planned. Read the code, report a bug or help on
        <a :href="GITHUB">GitHub</a>.
      </p>
      <div class="actions">
        <RouterLink to="/download" class="button primary">Download</RouterLink>
        <a :href="GITHUB" class="button">GitHub</a>
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
