import { defineConfig, lazyPlugins } from "vite-plus";
import { resolve } from "path";
import { globSync } from "tinyglobby";
import path from "node:path";
import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";

const root = path.dirname(fileURLToPath(import.meta.url));

const files = globSync("src/**/*.{js,ts,html}", {
  cwd: root,
  absolute: true,
  // skip type declarations and partials like _foo.css
  // ignore: ["**/*.d.ts", "**/_*"],
  ignore: ["**/*.d.ts"],
}).map((file) => [
  path
    .relative(path.join(root, "src"), file)
    .slice(0, -path.extname(file).length)
    .split(path.sep)
    .join("/"),
  file,
]);

if (files.length === 0) {
  throw new Error(`No entry files found under ${path.join(root, "src")}`);
}

const outDir = process.env.OUT_DIR || `dist`;

export default defineConfig({
  fmt: {
    ignorePatterns: ["public/vendored", "src/templates/**/*.html"],
  },
  lint: {
    jsPlugins: [{ name: "vite-plus", specifier: "vite-plus/oxlint-plugin" }],
    rules: { "vite-plus/prefer-vite-plus-imports": "error" },
    options: { typeAware: true, typeCheck: true },
    ignorePatterns: ["public/vendored"],
  },
  root,
  base: "/static/vite/",
  plugins: lazyPlugins(() => [tailwindcss()]),
  mode: "production",
  build: {
    outDir,
    emptyOutDir: true,
    // sourcemap: true,
    manifest: true,
    rolldownOptions: {
      input: Object.fromEntries(files),

      output: {
        minify: process.env.NODE_ENV === "production",
        // minify: false,
        // predictable filenames instead of hashed ones, so Askama
        // templates can reference /static/counter.js directly
        // entryFileNames: "[name].js",
        // assetFileNames: "[name].[ext]",
        //
        // entryFileNames: "[name].js", // stable name, no hash
        // chunkFileNames: "chunks/[name]-[hash].js",
        // assetFileNames: "assets/[name][extname]",
      },
    },
    resolve: {
      alias: {
        "@common": resolve(import.meta.dirname, "src/common"),
        "@src": resolve(import.meta.dirname, "src"),
      },
    },
  },
});
