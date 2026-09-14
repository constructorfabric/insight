import type { MembersGridMember } from "@/components/widgets/dashboard/members-grid";

export type MemberSelection =
  | { kind: "all" }
  | { kind: "selected"; ids: ReadonlySet<string> };

export function selectedMembers(
  members: readonly MembersGridMember[],
  selection: MemberSelection,
): MembersGridMember[] {
  if (selection.kind === "all") return [...members];
  return members.filter((member) => selection.ids.has(member.entityId));
}
