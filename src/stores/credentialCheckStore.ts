import { create } from 'zustand';
import type { CheckedCredential } from '../types';

type CredentialMenuState = { libraryId: string } & (
  | { outcome: 'checking' | 'unavailable' }
  | CheckedCredential
);

/** Transient results. The native command owns the saved identity and revision. */
export const useCredentialChecks = create<ReadonlyMap<string, CredentialMenuState>>(() => new Map());

function setCheck(id: string, value?: CredentialMenuState) {
  useCredentialChecks.setState(previous => {
    const next = new Map(previous);
    if (value) next.set(id, value);
    else next.delete(id);
    return next;
  }, true);
}

export async function runCredentialCheck(
  id: string, libraryId: string, request: (id: string) => Promise<CheckedCredential | null>,
) {
  if (useCredentialChecks.getState().get(id)?.outcome === 'checking') return;
  const pending = { libraryId, outcome: 'checking' as const };
  setCheck(id, pending);
  try {
    const result = await request(id);
    if (useCredentialChecks.getState().get(id) !== pending) return;
    setCheck(id, result?.connectionId === id && result.revision ? { ...result, libraryId } : undefined);
  } catch {
    if (useCredentialChecks.getState().get(id) === pending) setCheck(id, { libraryId, outcome: 'unavailable' });
  }
}

export function invalidateCredentialCheck(id: string) {
  if (useCredentialChecks.getState().has(id)) setCheck(id);
}

/** A reload can replace a profile while retaining its ID. */
export function invalidateLibraryCredentialChecks(libraryId?: string) {
  const checks = useCredentialChecks.getState();
  if (!checks.size) return;
  const remaining = libraryId === undefined ? new Map() : new Map([...checks].filter(([, entry]) => entry.libraryId !== libraryId));
  useCredentialChecks.setState(remaining, true);
}
