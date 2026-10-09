<script setup lang="ts">
import { computed } from 'vue'
import { useLoad } from '@/load'
import { setTitle } from '@/router'
import { date, size } from '@/format'
import { INTIFACE, RELEASES, REPOSITORY, USER_GUIDE } from '@/links'

setTitle('Download')

interface Asset {
  name: string
  size: number
  browser_download_url: string
}

interface Release {
  tag_name: string
  html_url: string
  draft: boolean
  prerelease: boolean
  published_at: string
  assets: Asset[]
}

// The newest release, alpha ones included (only those exist yet), as the app's updater sees them.
const release = useLoad(async () => {
  const response = await fetch(`https://api.github.com/repos/${REPOSITORY}/releases?per_page=10`)
  if (!response.ok) throw new Error(response.statusText)
  const releases: Release[] = await response.json()
  const latest = releases.find((r) => !r.draft)
  if (!latest) throw new Error('no release yet')
  return latest
})

const systems = [
  { name: 'Ubuntu 24.04+, Debian 13+, Mint 22+', suffix: '.deb', install: 'sudo apt install ./gameviber_*.deb' },
  { name: 'Fedora 40+', suffix: '.rpm', install: 'sudo dnf install ./gameviber-*.rpm' },
  { name: 'Arch, CachyOS, Manjaro', suffix: '.pkg.tar.zst', install: 'sudo pacman -U gameviber-*.pkg.tar.zst' },
  { name: 'SteamOS, Bazzite, other systems', suffix: '.tar.gz', install: 'Extract it and run ./gameviber (see its README.txt)' },
]

const files = computed(() =>
  systems.map((system) => ({
    ...system,
    asset: release.data.value?.assets.find((a) => a.name.endsWith(system.suffix)),
  })),
)
const sums = computed(() => release.data.value?.assets.find((a) => a.name === 'SHA256SUMS'))
</script>

<template>
  <div class="wrap page">
    <p class="eyebrow">Download</p>
    <h1>GameViber for Linux</h1>
    <p class="lead">
      <template v-if="release.data.value">
        Version <strong class="version">{{ release.data.value.tag_name.replace(/^v/, '') }}</strong>, released
        {{ date(release.data.value.published_at) }}.
        <span v-if="release.data.value.prerelease" class="tag accent">alpha</span>
      </template>
      <template v-else>Pick the file for your system.</template>
    </p>

    <p v-if="release.error.value" class="card notice">
      The release list cannot be read right now: get the files from the
      <a :href="RELEASES">releases page on GitHub</a>.
    </p>

    <ul class="files">
      <li v-for="file in files" :key="file.suffix" class="card file">
        <div class="what">
          <strong>{{ file.name }}</strong>
          <code class="muted">{{ file.install }}</code>
        </div>
        <a v-if="file.asset" :href="file.asset.browser_download_url" class="button primary">
          {{ file.suffix }} <span class="weight">{{ size(file.asset.size) }}</span>
        </a>
        <a v-else :href="RELEASES" class="button">{{ file.suffix }}</a>
      </li>
    </ul>
    <p class="muted small">
      <a v-if="sums" :href="sums.browser_download_url">SHA256SUMS</a>
      <template v-if="sums"> · </template>
      <a :href="release.data.value?.html_url ?? RELEASES">Release notes</a> ·
      <a :href="RELEASES">All releases</a>
    </p>
    <p class="muted">GameViber tells you when a new version is out and, with these files, installs it for you.</p>

    <section class="section">
      <h2>You also need</h2>
      <div class="grid">
        <div class="card">
          <h3><a :href="INTIFACE">Intiface Central</a></h3>
          <p class="muted">It connects your toys. Start it and click <strong>Start Server</strong> before GameViber.</p>
        </div>
        <div class="card">
          <h3>A gamepad and PipeWire</h3>
          <p class="muted">
            Any gamepad, for the rumble. PipeWire, for the game's sound: it is the default on recent systems.
          </p>
        </div>
      </div>
    </section>

    <section>
      <h2>First steps</h2>
      <ol class="first-steps">
        <li>Start Intiface Central and click <strong>Start Server</strong>.</li>
        <li>
          Start <strong>GameViber</strong> from your applications menu. Not with <code>sudo</code>: it asks for your
          password itself when it needs it.
        </li>
        <li>Check that your toys show up in <strong>Toys</strong>, and your gamepad in <strong>Setup</strong>.</li>
        <li>
          Get a mode for your game: from <RouterLink to="/games">the community</RouterLink>, or in
          <strong>Library › Create a mode</strong>, written by an AI assistant.
        </li>
        <li>Start your game and play. <strong>STOP ALL</strong>, or BACK + START held on the gamepad, stops everything.</li>
      </ol>
      <p><a :href="USER_GUIDE">Read the full guide ›</a></p>
    </section>
  </div>
</template>

<style scoped>
.page {
  padding-top: 32px;
}

.version {
  color: var(--text);
}

.notice {
  border-color: var(--danger);
  margin: 16px 0;
}

.files {
  list-style: none;
  padding: 0;
  margin: 20px 0 12px;
  display: grid;
  gap: 12px;
}

.file {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.what {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.what code {
  overflow-wrap: anywhere;
  font-size: 13px;
}

.weight {
  font-weight: 400;
  font-size: 13px;
  opacity: 0.8;
}

.small {
  font-size: 14px;
}

.first-steps {
  padding-left: 20px;
}

.first-steps li {
  margin-bottom: 10px;
}

@media (min-width: 640px) {
  .file {
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
  }

  .file .button {
    flex: none;
    min-width: 170px;
  }
}
</style>
