import * as React from "react"

const MOBILE_BREAKPOINT = 768
const MQ = `(max-width: ${MOBILE_BREAKPOINT - 1}px)`

function subscribe(cb: () => void): () => void {
  const mql = window.matchMedia(MQ)
  mql.addEventListener("change", cb)
  return () => mql.removeEventListener("change", cb)
}

function getSnapshot(): boolean {
  return window.matchMedia(MQ).matches
}

// SSR fallback — assume desktop. Hydration corrects on first mount.
function getServerSnapshot(): boolean {
  return false
}

export function useIsMobile(): boolean {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot)
}
