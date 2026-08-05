/**
 * Storybook config (see docs/testing/storybook-component-tests.md).
 *
 * The MSW worker (`public/mockServiceWorker.js`) is served automatically —
 * Storybook serves `public/` as a static dir by default.
 */

import type { StorybookConfig } from "@storybook/react-vite";

const config: StorybookConfig = {
  stories: ["../src/**/*.stories.@(ts|tsx)"],
  addons: [
    "@storybook/addon-themes",
    "@storybook/addon-vitest",
    "@storybook/addon-a11y",
    "@storybook/addon-docs",
  ],
  framework: {
    name: "@storybook/react-vite",
    options: {},
  },
  core: {
    disableTelemetry: true,
    enableCrashReports: false,
  },
  typescript: {
    reactDocgen: "react-docgen-typescript",
  },
  // Serve `public/` explicitly (Vite's default too) so the MSW worker at
  // public/mockServiceWorker.js is reachable in both `storybook dev` and the
  // addon-vitest browser run. Path is relative to this config dir.
  staticDirs: ["../public"],
};

export default config;
