<script setup lang="ts">
import { setTitle } from '@/router'

setTitle('Page not found')
</script>

<template>
  <div class="wrap state">
    <h1>Page not found</h1>
    <p>This page does not exist, or not anymore.</p>
    <div class="actions" style="justify-content: center">
      <RouterLink to="/" class="button">Home</RouterLink>
      <RouterLink to="/games" class="button">Community modes</RouterLink>
    </div>
  </div>
</template>
