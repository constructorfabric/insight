export function saveError(detail: string): {
  summary: string;
  details: string;
} {
  if (detail.startsWith("The request is saved but not applied:")) {
    return {
      summary:
        "Request saved but not applied; restoring the previous schedule failed.",
      details: detail,
    };
  }
  if (detail.includes("Reset requires the deployment schedule annotation")) {
    return {
      summary: "Schedule not changed: run reconcile before resetting.",
      details: detail,
    };
  }
  if (!detail.startsWith("The schedule was not changed:")) {
    return {
      summary: detail || "Could not save the schedule. Try again.",
      details: "",
    };
  }
  let summary = "Schedule not changed. Try again.";
  if (detail.includes("the cluster returned an error")) {
    summary = "Schedule not changed: the cluster returned an error.";
  } else if (
    detail.includes("did not answer in time") ||
    detail.includes("timed out")
  ) {
    summary = "Schedule not changed: the cluster did not answer in time.";
  } else if (detail.includes("the cluster refused the change")) {
    summary = "Schedule not changed: the cluster refused the change.";
  }
  return { summary, details: detail };
}
