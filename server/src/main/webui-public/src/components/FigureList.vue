<script setup lang="ts">
import type { Figures } from '@/api'

// What players who share their stats did with a mode (`Stats.Figures`). The number of
// players and of downloads stays hidden while they are small: they put newcomers off.
defineProps<{ figures: Figures }>()
</script>

<template>
  <dl class="figures">
    <div v-if="figures.players > 0">
      <dt>Played</dt>
      <dd>{{ figures.medianMinutes }} min</dd>
    </div>
    <div v-if="figures.players > 0">
      <dt>Came back</dt>
      <dd>{{ Math.round(figures.cameBack * 100) }} %</dd>
    </div>
    <div v-if="figures.likes > 0">
      <dt>Likes</dt>
      <dd>
        {{ figures.likes.toLocaleString('en') }}
        <span v-if="figures.dislikes" class="muted">/ {{ (figures.likes + figures.dislikes).toLocaleString('en') }}</span>
      </dd>
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
