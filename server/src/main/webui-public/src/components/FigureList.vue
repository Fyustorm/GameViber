<script setup lang="ts">
import type { Figures } from '@/api'

// What players who share their stats did with a mode (`Stats.Figures`).
defineProps<{ figures: Figures; downloads: number }>()
</script>

<template>
  <dl class="figures">
    <div>
      <dt>Players</dt>
      <dd>{{ figures.players.toLocaleString('en') }}</dd>
    </div>
    <div v-if="figures.players > 0">
      <dt>Played</dt>
      <dd>{{ figures.medianMinutes }} min</dd>
    </div>
    <div v-if="figures.players > 0">
      <dt>Came back</dt>
      <dd>{{ Math.round(figures.cameBack * 100) }} %</dd>
    </div>
    <div>
      <dt>Likes</dt>
      <dd>
        {{ figures.likes.toLocaleString('en') }}
        <span v-if="figures.dislikes" class="muted">/ {{ (figures.likes + figures.dislikes).toLocaleString('en') }}</span>
      </dd>
    </div>
    <div>
      <dt>Downloads</dt>
      <dd>{{ downloads.toLocaleString('en') }}</dd>
    </div>
  </dl>
</template>

<style scoped>
.figures {
  display: flex;
  flex-wrap: wrap;
  gap: 8px 20px;
  margin: 0;
}

dt {
  font-size: 12px;
  color: var(--muted);
}

dd {
  margin: 0;
  font-weight: 600;
}
</style>
