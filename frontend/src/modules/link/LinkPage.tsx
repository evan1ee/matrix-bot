import React, { useEffect, useState } from 'react';
import { Loader2 } from 'lucide-react';
import { fetchApi } from '../../core/apiClient';
import {
  describePasskeyError,
  isPasskeySupported,
  signInWithPasskey,
  signUpWithLink,
} from '../../core/passkey';
import { channelLabel, SECURITY_KEYS, useSensitiveAction } from '../../core/security';
import { useRefreshSession } from '../../core/session';
import type { components } from '../../types/api';

type Preview = components['schemas']['LinkPreviewDto'];
type ClaimOutcome = components['schemas']['LinkClaimOutcome'];

/** The query-string token, read once; the address bar is scrubbed by an effect. */
function readToken(): string {
  return new URLSearchParams(window.location.search).get('token')?.trim() ?? '';
}

const EXPIRED: Preview = { state: 'expired' };

const primaryButton =
  'inline-flex items-center justify-center gap-2 rounded-lg bg-neutral-900 px-4 py-2 text-sm font-medium text-white transition-colors duration-150 hover:bg-neutral-700 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-900 disabled:opacity-60 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-300 dark:focus-visible:outline-neutral-100';
const secondaryButton =
  'inline-flex items-center justify-center gap-2 rounded-lg border border-neutral-300 px-4 py-2 text-sm font-medium text-neutral-900 transition-colors duration-150 hover:bg-neutral-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-900 disabled:opacity-60 dark:border-neutral-700 dark:text-neutral-100 dark:hover:bg-neutral-800 dark:focus-visible:outline-neutral-100';

/**
 * Where the bot's `/link` message points.
 *
 * The token is a bearer secret until it is claimed, so it leaves the address
 * bar immediately. Everything the page offers is decided by the server's
 * inspect answer; this component only renders it and drives the matching call.
 * Claiming needs a fresh passkey assertion, which `useSensitiveAction` asks for
 * on the server's 403 and then retries.
 */
export const LinkPage: React.FC = () => {
  const [token] = useState(readToken);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [inspectNonce, setInspectNonce] = useState(0);
  const [wantsSignIn, setWantsSignIn] = useState(false);
  const [done, setDone] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const refreshSession = useRefreshSession();

  useEffect(() => {
    if (window.location.search) {
      window.history.replaceState(window.history.state, '', window.location.pathname);
    }
  }, []);

  useEffect(() => {
    if (!token) {
      setPreview(EXPIRED);
      return;
    }
    let cancelled = false;
    setPreview(null);
    fetchApi<Preview>('/link/inspect', {
      method: 'POST',
      body: JSON.stringify({ token }),
      // Signed-out is an expected answer here, not a session expiry.
      suppressAuthEvent: true,
    })
      .then((result) => {
        if (!cancelled) setPreview(result);
      })
      .catch((err) => {
        if (cancelled) return;
        setPreview(null);
        setError(describePasskeyError(err));
      });
    return () => {
      cancelled = true;
    };
  }, [token, inspectNonce]);

  const claim = useSensitiveAction(
    () =>
      fetchApi<{ outcome: ClaimOutcome }>('/link/claim', {
        method: 'POST',
        body: JSON.stringify({ token }),
      }),
    [SECURITY_KEYS.identities, SECURITY_KEYS.events]
  );

  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (err) {
      setError(describePasskeyError(err));
    } finally {
      setBusy(false);
    }
  };

  const onClaim = () =>
    run(async () => {
      const { outcome } = await claim.mutateAsync(undefined);
      if (outcome === 'linked' || outcome === 'merged') {
        setDone(true);
      } else {
        setPreview({ ...preview, state: outcome });
      }
    });

  const onSignIn = () =>
    run(async () => {
      await signInWithPasskey();
      refreshSession();
      setWantsSignIn(false);
      setInspectNonce((n) => n + 1);
    });

  const onSignUp = () =>
    run(async () => {
      await signUpWithLink(token);
      refreshSession();
      setDone(true);
    });

  const identity = preview?.channel
    ? `${channelLabel(preview.channel)} account ${preview.user_id ?? ''}`.trim()
    : 'this chat account';
  const supported = isPasskeySupported();

  const renderBody = (): React.ReactNode => {
    if (done) {
      return <p className="text-sm text-neutral-700 dark:text-neutral-300">Connected. You can close this tab.</p>;
    }
    if (!preview) {
      return error ? null : <Loader2 size={18} className="animate-spin text-neutral-500" aria-label="Loading" />;
    }

    const signInView = (heading: string) => (
      <form
        className="space-y-4"
        onSubmit={(e) => {
          e.preventDefault();
          void onSignIn();
        }}
      >
        <h2 className="text-lg font-semibold tracking-tight">{heading}</h2>
        <p className="text-sm text-neutral-600 dark:text-neutral-400">
          Sign in to your workspace first, then this link continues where it left off.
        </p>
        <button type="submit" autoFocus disabled={busy || !supported} className={primaryButton}>
          {busy && <Loader2 size={14} className="animate-spin" />}
          Sign in with a passkey
        </button>
      </form>
    );

    switch (preview.state) {
      case 'bind':
      case 'needs_merge': {
        const merge = preview.state === 'needs_merge';
        const count = preview.merge_item_count ?? 0;
        return (
          <form
            className="space-y-4"
            onSubmit={(e) => {
              e.preventDefault();
              void onClaim();
            }}
          >
            <h2 className="text-lg font-semibold tracking-tight">
              {merge ? `Merge ${identity} into your workspace?` : `Connect ${identity} to your workspace?`}
            </h2>
            {merge && (
              <p className="text-sm text-neutral-600 dark:text-neutral-400">
                This account already has its own workspace with {count} {count === 1 ? 'item' : 'items'}.
                Connecting it moves everything in it into yours. This cannot be undone.
              </p>
            )}
            <button type="submit" autoFocus disabled={busy} className={primaryButton}>
              {busy && <Loader2 size={14} className="animate-spin" />}
              {merge ? 'Merge and connect' : 'Connect'}
            </button>
          </form>
        );
      }
      case 'sign_in_required':
        return signInView(`Connect ${identity}`);
      case 'signup_allowed':
        if (wantsSignIn) return signInView(`Connect ${identity}`);
        return (
          <div className="space-y-4">
            <h2 className="text-lg font-semibold tracking-tight">Create your workspace</h2>
            <p className="text-sm text-neutral-600 dark:text-neutral-400">
              {identity} is not connected to a workspace yet. A passkey is how you will sign in here.
            </p>
            <div className="flex flex-wrap gap-3">
              <button
                type="button"
                autoFocus
                disabled={busy || !supported}
                onClick={() => void onSignUp()}
                className={primaryButton}
              >
                {busy && <Loader2 size={14} className="animate-spin" />}
                Create an account with a passkey
              </button>
              <button type="button" disabled={busy} onClick={() => setWantsSignIn(true)} className={secondaryButton}>
                I already have an account
              </button>
            </div>
          </div>
        );
      case 'already_yours':
        return (
          <p className="text-sm text-neutral-700 dark:text-neutral-300">
            {identity} is already connected to your workspace. Nothing to do.
          </p>
        );
      case 'owned_elsewhere':
        return (
          <p className="text-sm text-neutral-700 dark:text-neutral-300">
            {identity} is already connected to a different workspace. Unlink it there first, then send /link to
            the bot again.
          </p>
        );
      case 'expired':
      default:
        return (
          <p className="text-sm text-neutral-700 dark:text-neutral-300">
            This link has expired or was already used. Send /link to the bot again.
          </p>
        );
    }
  };

  return (
    <div className="min-h-screen bg-[var(--color-canvas)] font-sans text-[var(--color-text-primary)]">
      <div className="mx-auto max-w-md space-y-6 px-6 py-12">
        <header className="flex items-center gap-2 border-b border-neutral-200 pb-4 dark:border-neutral-800">
          <span className="text-sm font-semibold tracking-tight">Matrix Bot</span>
          <span className="font-mono text-xs text-neutral-400">/</span>
          <span className="text-xs text-neutral-500">Connect a chat account</span>
        </header>
        <main className="space-y-6">
          {renderBody()}
          {error && (
            <p role="alert" className="rounded-lg bg-red-50 p-3 text-sm text-red-700 dark:bg-red-950 dark:text-red-200">
              {error}
            </p>
          )}
        </main>
      </div>
    </div>
  );
};
