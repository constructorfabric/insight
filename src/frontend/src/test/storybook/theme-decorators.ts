/**
 * Theme toolbar for stories, matching Insight's class convention: our
 * `ThemeProvider` (src/components/theme-provider.tsx) toggles `light`/`dark`
 * classes on `<html>` (Tailwind / shadcn convention).
 */

import { withThemeByClassName } from "@storybook/addon-themes";
import type { Decorator } from "@storybook/react-vite";

export const themeDecorators: Decorator[] = [
  withThemeByClassName({
    themes: { light: "light", dark: "dark" },
    defaultTheme: "light",
  }),
];
