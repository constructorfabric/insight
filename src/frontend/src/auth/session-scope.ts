import type { Session } from "@/auth/types";

export function sessionAuthorizationScope(
  session: Session | null
): string | null {
  if (!session) return null;
  return JSON.stringify({
    tenantId: session.tenantId,
    personId: session.personId,
    impersonatorEmail: session.impersonatorEmail,
    roles: [...session.roles].sort(),
  });
}
