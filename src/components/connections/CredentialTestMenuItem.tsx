import { DropdownMenuItem } from '@/components/ui/dropdown-menu';
import { testSavedCredentials } from '../../lib/tauri';
import { runCredentialCheck, useCredentialChecks } from '../../stores/credentialCheckStore';
import { useLibraryStore } from '../../stores/libraryStore';
import type { CredentialCheckReason } from '../../types';

const reasons: Record<CredentialCheckReason, string> = {
  timeout: 'The check timed out after 30 seconds.',
  unknownHost: 'The saved host no longer exists.',
  hostChanged: 'The saved host changed during the check.',
  storeUnavailable: 'The saved host library is unavailable.',
  keyringUnavailable: 'The system credential store is unavailable.',
  unreachable: 'The host could not be reached.',
  negotiation: 'The host could not negotiate a secure RDP connection.',
  untrustedPeer: 'Choose Review server identity to verify the fingerprint, then test again.',
  missingIdentity: 'The saved host has no username.',
  missingCredential: 'No password is saved for this host.',
  authorizationDenied: 'The server denied access without confirming the password.',
  noServerConfirmation: 'The server did not confirm the credential.',
  serverStatus: 'The server returned an inconclusive sign-in status.',
  interactiveSignInRequired: 'The server requires interactive sign-in.',
  authentication: 'The authentication exchange could not be completed.',
  unsupportedAuthentication: 'The server did not negotiate a supported authentication method.',
  malformedResponse: 'The server returned an unreadable response.',
  transportClosed: 'The connection closed during the check.',
};
const labels = {
  checking: 'Checking', accepted: 'Accepted', rejected: 'Rejected',
  unableToVerify: 'Unable to verify', unavailable: 'Unable to verify',
};

export function CredentialTestMenuItem({ id, name }: { id: string; name: string }) {
  const check = useCredentialChecks(state => state.get(id));
  const reason = check?.outcome === 'unableToVerify' ? reasons[check.reason]
    : check?.outcome === 'unavailable' ? 'The app could not complete the check.' : undefined;
  const color = check?.outcome === 'accepted' ? 'var(--color-success)'
    : check?.outcome === 'rejected' ? 'var(--color-danger)' : 'var(--color-text-muted)';
  return <>
    <DropdownMenuItem
      aria-disabled={check?.outcome === 'checking'}
      onKeyDown={event => {
        if (event.repeat && (event.key === 'Enter' || event.key === ' ')) event.preventDefault();
      }}
      onSelect={event => {
        event.preventDefault(); // Keep this host's menu open to announce the result.
        const host = useLibraryStore.getState().tree.connections.find(connection => connection.id === id);
        if (host) void runCredentialCheck(id, host.libraryId, testSavedCredentials);
      }}
    >Test credentials</DropdownMenuItem>
    <p role="status" aria-atomic="true" className={check ? 'max-w-64 px-2 py-1 text-xs' : 'sr-only'} style={{ color }}>
      {check && <>{name} · {labels[check.outcome]}{reason && <span className="block">{reason}</span>}</>}
    </p>
  </>;
}
