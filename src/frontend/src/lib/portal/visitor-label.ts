export interface VisitorNaming {
  person_id: string;
  display_name: string;
  username?: string;
}

export interface VisitorLabel {
  label: string;
  detail: string;
}

export function visitorLabel(visitor: VisitorNaming): VisitorLabel {
  const name = visitor.display_name.trim();
  const handle = visitor.username?.trim() ?? "";
  const detail = handle ? `username: ${handle}` : visitor.person_id;

  return { label: name || handle || visitor.person_id, detail };
}
