import { useMutation } from "@tanstack/react-query";

import {
  downloadReport,
  previewReport,
  type ReportExportFormat,
  type ReportRecipe,
} from "@/api/reports-client";

export interface ReportPreviewInput {
  recipe: ReportRecipe;
  signal?: AbortSignal;
}

export interface ReportExportInput extends ReportPreviewInput {
  format: ReportExportFormat;
}

export function useReportPreview() {
  return useMutation({
    mutationFn: ({ recipe, signal }: ReportPreviewInput) =>
      previewReport(recipe, signal),
  });
}

export function useReportExport() {
  return useMutation({
    mutationFn: ({ recipe, format, signal }: ReportExportInput) =>
      downloadReport(recipe, format, signal),
  });
}
