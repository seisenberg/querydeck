import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri expects a fixed dev port and no auto-open.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: Number(process.env.PORT) || 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "es2021",
    sourcemap: false,
    // node-sql-parser's grammars (~2.6 MB) and an-array-of-english-words
    // (~3.3 MB) are code-split into their own async chunks (see lint.ts) and
    // only fetched on the first lint pass — never on the startup path. Raise
    // the size-warning threshold just above them so the build stays quiet
    // while still flagging anything genuinely oversized.
    chunkSizeWarningLimit: 3500,
    rollupOptions: {
      output: {
        // Split the large, stable vendor libraries out of the app chunk so
        // they cache across updates and the main chunk stays small.
        manualChunks(id) {
          if (!id.includes("node_modules")) return;
          if (/[\\/]node_modules[\\/](react|react-dom|scheduler)[\\/]/.test(id)) {
            return "react";
          }
          if (id.includes("@codemirror") || id.includes("@lezer")) {
            return "codemirror";
          }
          // node-sql-parser, an-array-of-english-words and sql-formatter are
          // reached only via dynamic import(), so Rollup already emits them as
          // their own lazy chunks — leave them to the default handling.
        },
      },
    },
  },
});
