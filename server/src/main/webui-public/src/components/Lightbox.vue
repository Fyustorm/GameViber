<script setup lang="ts">
import { onBeforeUnmount, onMounted } from 'vue'

// Images seen one at a time over the page: arrows or the keyboard to go through them, Escape to close.
const props = defineProps<{ images: { src: string; caption: string }[] }>()
const index = defineModel<number | null>({ required: true })

function step(by: number) {
  if (index.value === null) return
  index.value = (index.value + by + props.images.length) % props.images.length
}

function key(event: KeyboardEvent) {
  if (index.value === null) return
  if (event.key === 'Escape') index.value = null
  else if (event.key === 'ArrowLeft') step(-1)
  else if (event.key === 'ArrowRight') step(1)
}

onMounted(() => window.addEventListener('keydown', key))
onBeforeUnmount(() => window.removeEventListener('keydown', key))
</script>

<template>
  <div v-if="index !== null && images[index]" class="lightbox" role="dialog" aria-modal="true" @click.self="index = null">
    <figure>
      <img :src="images[index].src" alt="" />
      <figcaption>
        <span>{{ images[index].caption }}</span>
        <span class="muted">{{ index + 1 }} / {{ images.length }}</span>
      </figcaption>
    </figure>
    <button type="button" class="close" aria-label="Close" @click="index = null">×</button>
    <template v-if="images.length > 1">
      <button type="button" class="arrow previous" aria-label="Previous" @click="step(-1)">‹</button>
      <button type="button" class="arrow next" aria-label="Next" @click="step(1)">›</button>
    </template>
  </div>
</template>

<style scoped>
.lightbox {
  position: fixed;
  inset: 0;
  z-index: 50;
  display: grid;
  place-items: center;
  padding: 56px 12px;
  background: rgb(10 11 14 / 0.94);
}

figure {
  margin: 0;
  width: min(100%, 1400px);
}

img {
  width: 100%;
  max-height: calc(100vh - 140px);
  object-fit: contain;
  border-radius: 8px;
}

figcaption {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  margin-top: 10px;
  font-size: 14px;
}

button {
  position: absolute;
  width: 44px;
  height: 44px;
  border: 0;
  border-radius: 50%;
  background: var(--raised);
  color: var(--text);
  font-size: 26px;
  line-height: 1;
  cursor: pointer;
}

.close {
  top: 8px;
  right: 8px;
}

.arrow {
  top: 50%;
  translate: 0 -50%;
}

.previous {
  left: 8px;
}

.next {
  right: 8px;
}
</style>
