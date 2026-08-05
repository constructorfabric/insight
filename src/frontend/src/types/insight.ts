export type PeriodValue = "week" | "month" | "quarter" | "year";
export type CustomRange = { from: string; to: string };

export interface TeamMember {
  /** Canonical person id (NOT an email — the identity cutover key). */
  person_id: string;
  name: string;
}

/** Lightweight projection of the C# PersonResponse shape. */
export interface IdentityPerson {
  person_id: string;
  email: string;
  display_name: string;
  first_name?: string;
  last_name?: string;
  department?: string;
  division?: string;
  job_title?: string;
  status?: string;
  parent_email?: string | null;
  parent_id?: string | null;
  parent_person_id?: string | null;
  supervisor_email?: string | null;
  supervisor_name?: string | null;
  subordinates: IdentityPerson[];
}
