<script setup lang="ts">
import { computed } from 'vue'
import { useLoad } from '@/load'
import { setTitle } from '@/router'
import { date, size } from '@/format'
import { INTIFACE, RELEASES, REPOSITORY, USER_GUIDE } from '@/links'
import { visitorSystem, type System } from '@/system'

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

interface File {
  name: string
  suffix: string
  // A command to type, or what to do in words.
  command?: string
  how?: string
}

// The files `packaging/windows/package.sh` and `packaging/linux/package.sh` make.
const platforms: { system: System; title: string; files: File[] }[] = [
  {
    system: 'windows',
    title: 'Windows 10 (2004) or later',
    files: [
      {
        name: 'Installer',
        suffix: '-setup.exe',
        how: 'Run it: it also offers ViGEmBus, which GameViber needs to capture the rumble, and HidHide.',
      },
      {
        name: 'Without installing',
        suffix: '.zip',
        how: 'Extract it and run gameviber.exe. Install ViGEmBus yourself (see its README.txt).',
      },
    ],
  },
  {
    system: 'linux',
    title: 'Linux',
    files: [
      { name: 'Ubuntu 24.04+, Debian 13+, Mint 22+', suffix: '.deb', command: 'sudo apt install ./gameviber_*.deb' },
      { name: 'Fedora 40+', suffix: '.rpm', command: 'sudo dnf install ./gameviber-*.rpm' },
      { name: 'Arch, CachyOS, Manjaro', suffix: '.pkg.tar.zst', command: 'sudo pacman -U gameviber-*.pkg.tar.zst' },
      { name: 'SteamOS, Bazzite, other systems', suffix: '.tar.gz', how: 'Extract it and run ./gameviber (see its README.txt).' },
    ],
  },
]

// The visitor's system first.
const system = visitorSystem()
const ordered = [...platforms].sort((a, b) => Number(b.system === system) - Number(a.system === system))

const sections = computed(() =>
  ordered.map((platform) => ({
    ...platform,
    files: platform.files.map((file) => ({
      ...file,
      asset: release.data.value?.assets.find((a) => a.name.endsWith(file.suffix)),
    })),
  })),
)
// alpha, beta, rc: the release's stage, from its version.
const stage = computed(() => release.data.value?.tag_name.match(/-([a-z]+)/)?.[1] ?? null)
const sums = computed(() => release.data.value?.assets.find((a) => a.name === 'SHA256SUMS'))
</script>

<template>
  <div class="wrap page">
    <p class="eyebrow">Download</p>
    <h1>Download GameViber</h1>
    <p class="lead">
      <template v-if="release.data.value">
        Version <strong class="version">{{ release.data.value.tag_name.replace(/^v/, '') }}</strong>, released
        {{ date(release.data.value.published_at) }}.
        <span v-if="stage" class="tag accent">{{ stage }}</span>
      </template>
      <template v-else>Pick the file for your system.</template>
    </p>

    <p v-if="release.error.value" class="card notice">
      The release list cannot be read right now: get the files from the
      <a target="_blank" rel="noopener" :href="RELEASES">releases page on GitHub</a>.
    </p>

    <section v-for="platform in sections" :key="platform.system" class="platform">
      <h2>{{ platform.title }}</h2>
      <ul class="files">
        <li v-for="file in platform.files" :key="file.suffix" class="card file">
          <div class="what">
            <strong>{{ file.name }}</strong>
            <code v-if="file.command" class="muted">{{ file.command }}</code>
            <span v-else class="muted how">{{ file.how }}</span>
          </div>
          <a v-if="file.asset" target="_blank" rel="noopener" :href="file.asset.browser_download_url" class="button primary">
            {{ file.suffix.replace(/^-/, '') }} <span class="weight">{{ size(file.asset.size) }}</span>
          </a>
          <a v-else target="_blank" rel="noopener" :href="RELEASES" class="button">{{ file.suffix.replace(/^-/, '') }}</a>
        </li>
      </ul>
    </section>
    <p class="muted small">
      <a v-if="sums" target="_blank" rel="noopener" :href="sums.browser_download_url">SHA256SUMS</a>
      <template v-if="sums"> · </template>
      <a target="_blank" rel="noopener" :href="release.data.value?.html_url ?? RELEASES">Release notes</a> ·
      <a target="_blank" rel="noopener" :href="RELEASES">All releases</a>
    </p>
    <p class="muted">GameViber tells you when a new version is out and, with these files, installs it for you.</p>

    <section class="section">
      <h2>You also need</h2>
      <div class="grid">
        <div class="card">
          <h3><a target="_blank" rel="noopener" :href="INTIFACE">Intiface Central</a></h3>
          <p class="muted">It connects your toys. Start it and click <strong>Start Server</strong> before GameViber.</p>
        </div>
        <div class="card">
          <h3>A gamepad</h3>
          <p class="muted">
            Any gamepad, for the rumble. On Windows, ViGEmBus (the installer offers it); on Linux, PipeWire for the
            game's sound, the default on recent systems.
          </p>
        </div>
      </div>
    </section>

    <section>
      <h2>First steps</h2>
      <ol class="first-steps">
        <li>Start Intiface Central and click <strong>Start Server</strong>.</li>
        <li>
          Start <strong>GameViber</strong> from your applications menu. On Linux, not with <code>sudo</code>: it asks
          for your password itself when it needs it.
        </li>
        <li>Check that your toys show up in <strong>Toys</strong>, and your gamepad in <strong>Setup</strong>.</li>
        <li>
          Get a mode for your game: from <RouterLink to="/games">the community</RouterLink>, or in
          <strong>Library › Create a mode</strong>, written by an AI assistant.
        </li>
        <li>Start your game and play. <strong>STOP ALL</strong>, or BACK + START held on the gamepad, stops everything.</li>
      </ol>
      <p><a target="_blank" rel="noopener" :href="USER_GUIDE">Read the full guide ›</a></p>
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

.platform {
  margin-top: 28px;
}

.platform h2 {
  font-size: 20px;
}

.how {
  font-size: 14px;
}

.files {
  list-style: none;
  padding: 0;
  margin: 12px 0 12px;
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
