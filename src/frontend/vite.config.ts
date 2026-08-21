import babel from "@rolldown/plugin-babel";
import tailwindcss from "@tailwindcss/vite";
import { tanstackRouter } from "@tanstack/router-plugin/vite";
import react, { reactCompilerPreset } from "@vitejs/plugin-react";
import { execSync } from "child_process";
import path from "path";
import { defineConfig, loadEnv } from "vite";

import { backendProxy } from "./vite.backend-proxy";
import { uiKitLayer } from "./vite.ui-kit-layer";

// CI passes the image tag as VITE_APP_RELEASE. Everywhere else, name the
// release after the commit built from. `.dockerignore` drops `.git`, so this
// finds nothing inside the image build.
function localRelease(): string | undefined {
  try {
    const sha = execSync("git rev-parse --short HEAD", {
      stdio: ["ignore", "pipe", "ignore"],
    })
      .toString()
      .trim();
    return `local-${sha}`;
  } catch {
    return undefined;
  }
}

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const release = env.VITE_APP_RELEASE || localRelease();
  // Assigning undefined would store the literal string "undefined".
  if (release) process.env.VITE_APP_RELEASE = release;
  return {
    // Relative asset base: one image serves at "/" AND under an /exp/<name>
    // preview prefix. Correct resolution on nested routes comes from the
    // runtime <base href> injected in index.html.
    base: "./",
    plugins: [
      tanstackRouter({ target: "react", autoCodeSplitting: true }),
      react(),
      babel({ presets: [reactCompilerPreset()] }),
      tailwindcss(),
      uiKitLayer(),
      backendProxy(env),
    ],
    resolve: {
      alias: {
        "@": path.resolve(__dirname, "./src"),
      },
    },
  };
});
