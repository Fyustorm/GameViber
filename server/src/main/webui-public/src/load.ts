import { ref, shallowRef, watch, type WatchSource } from 'vue'

/** What `load` answers, loaded again whenever `source` changes; an older answer arriving late is dropped. */
export function useLoad<T>(load: () => Promise<T>, source?: WatchSource) {
  const data = shallowRef<T>()
  const error = ref<Error>()
  const loading = ref(true)
  let latest = 0

  async function run() {
    const mine = ++latest
    loading.value = true
    error.value = undefined
    try {
      const value = await load()
      if (mine === latest) data.value = value
    } catch (e) {
      if (mine === latest) error.value = e instanceof Error ? e : new Error(String(e))
    } finally {
      if (mine === latest) loading.value = false
    }
  }

  if (source) watch(source, run, { immediate: true })
  else run()
  return { data, error, loading, reload: run }
}
