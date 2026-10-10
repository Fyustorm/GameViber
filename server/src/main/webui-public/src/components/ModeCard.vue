<script setup lang="ts">
import { USES, type ModeSummary } from '@/api'
import { ago } from '@/format'

defineProps<{ mode: ModeSummary }>()
</script>

<template>
  <RouterLink :to="`/modes/${mode.id}`" class="mode card">
    <div class="head">
      <h3>{{ mode.name }}</h3>
      <span class="tag">v{{ mode.version }}</span>
    </div>
    <p class="by muted">by {{ mode.author }} · updated {{ ago(mode.updatedAt) }}</p>
    <p v-if="mode.uses?.length" class="uses">
      <span v-for="use in mode.uses" :key="use" class="tag accent" :title="USES[use].text">{{ USES[use].label }}</span>
    </p>
    <p v-if="mode.description" class="description">{{ mode.description }}</p>
    <p class="numbers muted">
      <span>{{ mode.figures.players.toLocaleString('en') }} players this month</span>
      <span v-if="mode.figures.likes">{{ mode.figures.likes.toLocaleString('en') }} likes</span>
      <span>{{ mode.downloads.toLocaleString('en') }} downloads</span>
    </p>
  </RouterLink>
</template>

<style scoped>
.head {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 12px;
}

h3 {
  margin: 0 0 4px;
}

.by {
  font-size: 14px;
}

.uses {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.description {
  display: -webkit-box;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
  overflow: hidden;
  white-space: pre-line;
}

.numbers {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 16px;
  font-size: 14px;
  margin: 0;
}
</style>
