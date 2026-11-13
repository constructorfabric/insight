import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

export function presetLabel(value: string): string {
  if (value.startsWith("hours-")) {
    const hours = Number(value.slice(6));
    return hours === 1 ? "Every hour" : `Every ${hours} hours`;
  }
  const labels: Record<string, string> = {
    daily: "Every day",
    weekdays: "Every weekday (Mon–Fri)",
    custom: "Custom cron (UTC)",
  };
  const label = labels[value];
  if (!label) throw new Error(`Unknown preset: ${value}`);
  return label;
}

export async function pickPreset(value: string) {
  const trigger = screen.getByRole("combobox", { name: "Runs" });
  await userEvent.click(trigger);
  await userEvent.click(
    await screen.findByRole("option", { name: presetLabel(value) })
  );
}
