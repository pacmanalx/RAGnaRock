import { defineConfig, loadEnv } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { fileURLToPath, URL } from 'node:url'

// Servidor autônomo AGORA, desacoplável DEPOIS: os alvos dos dois backends (ragd + nidhoggd)
// vêm de env — nunca hardcoded. Em dev o proxy do Vite evita CORS; em prod o ragd serve o
// dist/ na mesma origem. Para apontar pra outra máquina/cluster: só mudar as VITE_*_URL.
export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), '')
  const RAGD = env.VITE_RAGD_PROXY || 'http://127.0.0.1:11499'
  const NIDHOGG = env.VITE_NIDHOGG_PROXY || 'http://127.0.0.1:11497'
  return {
    plugins: [react(), tailwindcss()],
    resolve: {
      alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
    },
    server: {
      proxy: {
        '/api': { target: RAGD, changeOrigin: true, rewrite: (p) => p.replace(/^\/api/, '') },
        '/nidhogg': { target: NIDHOGG, changeOrigin: true, rewrite: (p) => p.replace(/^\/nidhogg/, '') },
      },
    },
  }
})
