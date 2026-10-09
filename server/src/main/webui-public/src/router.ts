import { createRouter, createWebHistory } from 'vue-router'
import HomeView from './views/HomeView.vue'

// Each route but the home page is listed in `SitePages` too, which serves the page on a
// direct visit, with its title and preview filled in.
export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', component: HomeView },
    { path: '/games', component: () => import('./views/GamesView.vue') },
    { path: '/games/:id', component: () => import('./views/GameView.vue'), props: (r) => ({ id: Number(r.params.id) }) },
    { path: '/modes/:id', component: () => import('./views/ModeView.vue'), props: (r) => ({ id: r.params.id }) },
    { path: '/m/:code', component: () => import('./views/ModeView.vue'), props: (r) => ({ code: r.params.code }) },
    { path: '/download', component: () => import('./views/DownloadView.vue') },
    { path: '/:rest(.*)*', component: () => import('./views/NotFoundView.vue') },
  ],
  scrollBehavior: (to, _from, saved) => saved ?? (to.hash ? { el: to.hash, top: 72 } : { top: 0 }),
})

const SITE = 'GameViber'

/** The tab's title once a page knows what it shows. */
export function setTitle(title?: string) {
  document.title = title ? `${title} · ${SITE}` : `${SITE}: feel your games on your toys`
}

router.afterEach((to) => {
  if (to.path === '/') setTitle()
})
