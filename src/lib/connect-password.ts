import type { ConnectionConfig } from '../types';
import { normalizeHostProfile } from './host-profile';
import * as tauri from './tauri';
import { useConnectionStore } from '../stores/connectionStore';
import { usePasswordPrompt } from '../stores/passwordPromptStore';

export const CONNECT_CANCELLED = 'Connection cancelled: no password was entered.';

/**
 * The password to send with Connect. `undefined` lets the backend read the
 * keychain. A host that does not save its password, or has none saved, is
 * asked for one; that password stays with the session and is saved only
 * when the user asks and the host allows it.
 */
export function connectPassword(connectionId: string): Promise<string | undefined> | undefined {
  const connection = useConnectionStore.getState().connections.find((c) => c.id === connectionId);
  // Unknown here: the backend resolves the saved connection itself.
  if (!connection) return undefined;
  return ask(connectionId, connection);
}

async function ask(connectionId: string, connection: ConnectionConfig): Promise<string | undefined> {
  const { savePassword } = normalizeHostProfile(connection.hostProfile);
  // An unreadable keychain falls back to the backend, which reports it.
  if (savePassword && await tauri.hasSavedPassword(connectionId).catch(() => true)) return undefined;
  const answer = await usePasswordPrompt.getState().ask({
    connectionId, name: connection.name, username: connection.username,
    reason: savePassword ? 'missing' : 'notSaved',
  });
  if (!answer) throw new Error(CONNECT_CANCELLED);
  if (answer.save && savePassword) {
    // Saving is a convenience; the connection proceeds either way.
    await useConnectionStore.getState().updateConnection(connectionId, { password: answer.password })
      .catch((error: unknown) => console.warn('Password not saved to the keychain', error));
  }
  return answer.password;
}
