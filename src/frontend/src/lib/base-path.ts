// The runtime <base> tag (injected first-in-head by index.html) is the single
// source of the serving prefix: "/" normally, "/exp/<name>/" inside a preview
// experiment. Everything that needs the prefix derives it from here.
export function servingBasepath(baseURI: string = document.baseURI): string {
  return new URL(baseURI).pathname;
}
