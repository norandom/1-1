import { create } from 'zustand';

/** Why 1-1 asks: the host does not save passwords, or the keychain has none for it. */
export type PasswordPromptReason = 'notSaved' | 'missing';

export interface PasswordAnswer {
  password: string;
  /** Store it in the keychain (offered only when the host saves passwords). */
  save: boolean;
}

export interface PasswordRequest {
  connectionId: string;
  name: string;
  username: string;
  reason: PasswordPromptReason;
  resolve: (answer: PasswordAnswer | null) => void;
}

interface PasswordPromptState {
  request: PasswordRequest | null;
  /** Ask for a password; null when the user cancels. One request at a time. */
  ask: (request: Omit<PasswordRequest, 'resolve'>) => Promise<PasswordAnswer | null>;
  answer: (answer: PasswordAnswer | null) => void;
}

export const usePasswordPrompt = create<PasswordPromptState>((set, get) => ({
  request: null,
  ask: (request) => new Promise((resolve) => {
    // A newer request replaces an unanswered one, which counts as cancelled.
    get().request?.resolve(null);
    set({ request: { ...request, resolve } });
  }),
  answer: (answer) => {
    const current = get().request;
    set({ request: null });
    current?.resolve(answer);
  },
}));
