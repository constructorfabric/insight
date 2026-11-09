import { personDisplayName } from "@/lib/identities/person-display";

export interface VisitorNaming {
  person_id: string;
  display_name: string;
  username?: string;
  email?: string | null;
}

export interface VisitorLabel {
  label: string;
  detail: string;
}

export function visitorLabel(visitor: VisitorNaming): VisitorLabel {
  const handle = visitor.username?.trim() ?? "";
  const detail = handle ? `username: ${handle}` : visitor.person_id;

  return { label: personDisplayName(visitor), detail };
}
