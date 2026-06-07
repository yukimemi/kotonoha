import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { VitePWA } from "vite-plugin-pwa";

// https://vite.dev/config/
export default defineConfig({
  plugins: [
    tailwindcss(),
    react(),
    VitePWA({
      // Disabled in dev: a stale service worker is the #1 source of
      // "blank page" mysteries on a fresh checkout.  Production
      // build still installs it.
      devOptions: { enabled: false },
      registerType: "autoUpdate",
      includeAssets: ["favicon.svg"],
      manifest: {
        name: "ことのは — kotonoha",
        short_name: "ことのは",
        description: "VTuber 先生と英会話の練習をしよう",
        lang: "ja",
        theme_color: "#FDF7EE",
        background_color: "#FDF7EE",
        display: "standalone",
        orientation: "any",
        start_url: "/",
        scope: "/",
        icons: [
          {
            src: "/favicon.svg",
            sizes: "any",
            type: "image/svg+xml",
            purpose: "any maskable",
          },
        ],
      },
      workbox: {
        // VRM files are large; let them stream from network and cache.
        globPatterns: ["**/*.{js,css,html,svg,png,ico,woff,woff2}"],
        navigateFallback: "/index.html",
        navigateFallbackDenylist: [/^\/api\//, /^\/avatars\//, /^\/ws\//],
      },
    }),
  ],
  server: {
    host: true, // LAN + Tailscale access for phone testing
    port: 5173,
    // Fail loudly if 5173 is taken instead of silently shifting to
    // 5174/5175/... — saves "why is my URL different now" confusion
    // when multiple dev sessions stack up.
    strictPort: true,
    allowedHosts: [".ts.net", ".local", "localhost"],
    proxy: {
      "/api":     { target: "http://127.0.0.1:7400", changeOrigin: true },
      "/avatars": { target: "http://127.0.0.1:7400", changeOrigin: true },
      "/ws":      { target: "ws://127.0.0.1:7400", ws: true, changeOrigin: true },
    },
  },
});
