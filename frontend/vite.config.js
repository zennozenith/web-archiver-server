import tailwindcss from "@tailwindcss/vite";
import { globSync } from "tinyglobby";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.dirname(fileURLToPath(import.meta.url));

const files = globSync("src/**/*.{js,ts,css}", {
  cwd: root,
  absolute: true,
  // skip type declarations and partials like _foo.css
  ignore: ["**/*.d.ts", "**/_*"],
});

if (files.length === 0) {
  throw new Error(`No entry files found under ${path.join(root, "src")}`);
}

/** @type {import('vite').UserConfig} */
export default {
  root,
  plugins: [tailwindcss()],
  mode: "production",
  build: {
    outDir: "../static/vite",
    emptyOutDir: true,
    rollupOptions: {
      input: Object.fromEntries(
        files.map((file) => [
          path
            .relative(path.join(root, "src"), file)
            .slice(0, -path.extname(file).length)
            .split(path.sep)
            .join("/"),
          file,
        ]),
      ),
      output: {
        // predictable filenames instead of hashed ones, so Askama
        // templates can reference /static/counter.js directly
        entryFileNames: "[name].js",
        assetFileNames: "[name].[ext]",
      },
    },
  },
};
