import babel from "@rolldown/plugin-babel";
import tailwindcss from "@tailwindcss/vite";
import { tanstackRouter } from "@tanstack/router-plugin/vite";
import react, { reactCompilerPreset } from "@vitejs/plugin-react";
import { execSync } from "child_process";
import path from "path";
import { defineConfig, loadEnv } from "vite";

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

export default defineConfig(({ mode, command }) => {
  const env = loadEnv(mode, process.cwd(), "");
  // Dev serve defaults to the compose gateway so /auth + /api always have an
  // upstream; without one, vite's SPA fallback serves /auth/login itself and
  // the login redirect loops. Builds never need a proxy.
  const proxyTarget =
    env.VITE_API_PROXY_TARGET ||
    (command === "serve" ? "http://localhost:8080" : undefined);
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
    ],
    resolve: {
      alias: {
        "@": path.resolve(__dirname, "./src"),
      },
    },
    server: proxyTarget
      ? {
          // Proxy both /api and /auth to the gateway dev target so the
          // cookie/BFF flow works under `pnpm dev`: /auth/login, /auth/me,
          // /auth/logout and all /api/* calls hit the same gateway origin.
          // changeOrigin keeps the upstream Host correct; the `__Host-sid`
          // cookie works over http://localhost because the proxy leaves the
          // response Set-Cookie untouched (no cookie-domain rewrite).
          proxy: {
            "/api": {
              target: proxyTarget,
              changeOrigin: true,
            },
            "/auth": {
              target: proxyTarget,
              changeOrigin: true,
            },
          },
        }
      : undefined,
  };
});
