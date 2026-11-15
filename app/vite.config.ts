import { defineConfig, type Plugin } from 'vite';
import react from '@vitejs/plugin-react';
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

// Файлы pdf.js для PDF без встроенных шрифтов и с редкими картинками: таблицы CMap, стандартные шрифты,
// декодеры (wasm) и цветовой профиль. Кладутся в сборку как есть (`pdfjs/…`), при разработке отдаются
// прямо из node_modules — копий в репозитории нет.
const PDFJS = fileURLToPath(new URL('./node_modules/pdfjs-dist/', import.meta.url));
const VERSION = JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf8')).version as string;
const PDFJS_DIRS = ['cmaps', 'standard_fonts', 'wasm', 'iccs'];

function pdfjsAssets(): Plugin {
  return {
    name: 'polka-pdfjs-assets',
    configureServer(server) {
      server.middlewares.use('/pdfjs/', (req, res, next) => {
        const parts = decodeURIComponent((req.url ?? '').split('?')[0]).split('/').filter(Boolean);
        if (parts.length !== 2 || !PDFJS_DIRS.includes(parts[0]) || parts[1].startsWith('.')) return next();
        const file = join(PDFJS, parts[0], parts[1]);
        if (!existsSync(file) || !statSync(file).isFile()) return next();
        res.setHeader('Content-Type', file.endsWith('.wasm') ? 'application/wasm' : file.endsWith('.js') ? 'text/javascript' : 'application/octet-stream');
        res.end(readFileSync(file));
      });
    },
    generateBundle() {
      for (const dir of PDFJS_DIRS) {
        // PDF со скриптами «Полка» не исполняет — движок скриптов (quickjs) не нужен.
        for (const name of readdirSync(join(PDFJS, dir)).filter((n) => !n.startsWith('quickjs'))) {
          this.emitFile({ type: 'asset', fileName: `pdfjs/${dir}/${name}`, source: readFileSync(join(PDFJS, dir, name)) });
        }
      }
    },
  };
}

// На телефоне при разработке Tauri подставляет адрес компьютера в TAURI_DEV_HOST.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react(), pdfjsAssets()],
  // Версия «Полки» для экрана настроек — из package.json.
  define: { __APP_VERSION__: JSON.stringify(VERSION) },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  build: {
    target: 'es2022',
  },
  // Поток pdf.js — модулем (как сам pdf.js).
  worker: { format: 'es' },
});
