<script setup lang="ts">
import { ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { GITHUB, USER_GUIDE } from './links'

const open = ref(false)
const route = useRoute()
watch(() => route.fullPath, () => (open.value = false))
</script>

<template>
  <header class="bar">
    <div class="bar-inner">
      <RouterLink to="/" class="brand" aria-label="GameViber, home">
        <img src="/favicon.svg" alt="" width="28" height="28" />
        <span>GameViber</span>
      </RouterLink>
      <button class="menu-button" type="button" :aria-expanded="open" aria-controls="menu" @click="open = !open">
        <span class="visually-hidden">Menu</span>
        <svg viewBox="0 0 24 24" width="24" height="24" aria-hidden="true">
          <path v-if="!open" d="M4 7h16M4 12h16M4 17h16" />
          <path v-else d="M6 6l12 12M18 6L6 18" />
        </svg>
      </button>
      <nav id="menu" :class="{ open }" aria-label="Main">
        <RouterLink to="/games">Modes</RouterLink>
        <a :href="USER_GUIDE">Guide</a>
        <a :href="GITHUB">GitHub</a>
        <RouterLink to="/download" class="button primary small">Download</RouterLink>
      </nav>
    </div>
  </header>

  <main>
    <RouterView />
  </main>

  <footer class="footer">
    <div class="footer-inner">
      <p>
        <strong>GameViber</strong> is free software (GPL-3.0), made by players. For adults only.
        Not affiliated with any game publisher or toy maker.
      </p>
      <nav aria-label="Footer">
        <RouterLink to="/download">Download</RouterLink>
        <RouterLink to="/games">Community modes</RouterLink>
        <a :href="USER_GUIDE">User guide</a>
        <a :href="GITHUB">Source code</a>
      </nav>
    </div>
  </footer>
</template>
