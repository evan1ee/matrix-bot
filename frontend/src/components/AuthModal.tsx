import React, { useEffect, useState } from 'react';
import { KeyRound, Loader2, ShieldCheck, X } from 'lucide-react';
import {
  describePasskeyError,
  enrollPasskey,
  isPasskeySupported,
  registerPasskey,
  signInWithPasskey,
  signOut,
} from '../core/passkey';
import { SessionUser } from '../core/session';

interface AuthModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSuccess: () => void;
  user: SessionUser | null;
  enrollToken?: string | null;
}

type Mode = 'signin' | 'register' | 'recovery' | 'enroll';

/**
 * Sign-in, registration, and sign-out.
 *
 * There is no username field. Signing in is one button: the browser offers the
 * passkeys it holds for this site, and the credential itself says which account
 * it belongs to. Registration asks only for a display name, which is cosmetic —
 * it never decides who owns anything.
 */
export const AuthModal: React.FC<AuthModalProps> = ({
  isOpen,
  onClose,
  onSuccess,
  user,
  enrollToken,
}) => {
  const [mode, setMode] = useState<Mode>('signin');
  const [displayName, setDisplayName] = useState('');
  const [invite, setInvite] = useState('');
  const [recoveryCode, setRecoveryCode] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Reopening always starts at sign-in or enroll if a token is present.
  useEffect(() => {
    if (isOpen) {
      setMode(enrollToken ? 'enroll' : 'signin');
      setError(null);
      setDisplayName('');
      setInvite('');
      setRecoveryCode('');
    }
  }, [isOpen, enrollToken]);

  useEffect(() => {
    if (!isOpen) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !busy) onClose();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [isOpen, busy, onClose]);

  if (!isOpen) return null;

  const supported = isPasskeySupported();

  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
      onSuccess();
      onClose();
    } catch (err) {
      setError(describePasskeyError(err));
    } finally {
      setBusy(false);
    }
  };

  const handleSignOut = () => run(signOut);
  const handleSignIn = () => run(signInWithPasskey);
  const handleRegister = () => {
    if (!displayName.trim()) {
      setError('Choose a name to show on your account.');
      return;
    }
    return run(() => registerPasskey(displayName.trim(), invite.trim() || undefined));
  };
  const handleRecover = () => {
    if (!recoveryCode.trim()) {
      setError('Please enter your recovery code.');
      return;
    }
    return run(async () => {
      const resp = await fetch('/api/v1/auth/recovery/redeem', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ code: recoveryCode.trim() }),
      });
      if (!resp.ok) {
        const errJson = await resp.json().catch(() => ({}));
        throw new Error(errJson.error || 'Failed to redeem recovery code');
      }
    });
  };

  const handleEnroll = () => {
    if (!enrollToken) return;
    return run(async () => {
      await enrollPasskey(enrollToken);
      if (typeof window !== 'undefined' && window.history?.replaceState) {
        const url = new URL(window.location.href);
        url.searchParams.delete('enroll');
        url.searchParams.delete('token');
        window.history.replaceState({}, '', url.pathname + (url.search ? url.search : '') + url.hash);
      }
    });
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="auth-title"
      onClick={(e) => {
        if (e.target === e.currentTarget && !busy) onClose();
      }}
    >
      <div className="w-full max-w-sm rounded-xl bg-white p-6 shadow-xl border border-neutral-200 dark:border-neutral-800 dark:bg-neutral-900">
        <div className="mb-5 flex items-start justify-between gap-4">
          <h2 id="auth-title" className="text-lg font-semibold text-neutral-900 dark:text-neutral-100">
            {user
              ? 'Your account'
              : mode === 'signin'
                ? 'Sign in'
                : mode === 'register'
                  ? 'Create an account'
                  : mode === 'enroll'
                    ? 'Link Passkey'
                    : 'Recover account'}
          </h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="rounded p-1 text-neutral-500 hover:bg-neutral-100 dark:hover:bg-neutral-800"
          >
            <X size={18} />
          </button>
        </div>

        {!supported && (
          <p className="mb-4 rounded-lg bg-amber-50 p-3 text-sm text-amber-800 dark:bg-amber-950 dark:text-amber-200">
            This browser does not support passkeys. Use a current version of Safari,
            Chrome, Edge or Firefox.
          </p>
        )}

        {error && (
          <p
            role="alert"
            className="mb-4 rounded-lg bg-red-50 p-3 text-sm text-red-700 dark:bg-red-950 dark:text-red-200"
          >
            {error}
          </p>
        )}

        {user ? (
          <div className="flex flex-col gap-4">
            <div className="flex items-center gap-3 rounded-lg bg-neutral-50 p-3 dark:bg-neutral-800">
              <ShieldCheck size={20} className="shrink-0 text-emerald-600 dark:text-emerald-400" />
              <div className="min-w-0">
                <p className="truncate font-medium text-neutral-900 dark:text-neutral-100">
                  {user.display_name || user.username}
                </p>
                <p className="truncate text-xs text-neutral-500 dark:text-neutral-400">
                  {user.linked_mxids.length > 0
                    ? user.linked_mxids.join(', ')
                    : 'Web account'}
                </p>
              </div>
            </div>

            <button
              type="button"
              onClick={handleSignOut}
              disabled={busy}
              className="flex items-center justify-center gap-2 rounded-lg bg-neutral-900 px-4 py-2.5 font-medium text-white disabled:opacity-60 dark:bg-neutral-100 dark:text-neutral-900"
            >
              {busy && <Loader2 size={16} className="animate-spin" />}
              Sign out
            </button>
          </div>
        ) : mode === 'enroll' ? (
          <div className="flex flex-col gap-4">
            <p className="text-sm text-neutral-600 dark:text-neutral-400">
              Link this device to your account using biometric authentication (Face ID, Touch ID, or Windows Hello).
            </p>

            <button
              type="button"
              onClick={handleEnroll}
              disabled={busy || !supported}
              className="flex items-center justify-center gap-2 rounded-lg bg-neutral-900 px-4 py-2.5 font-medium text-white disabled:opacity-60 dark:bg-neutral-100 dark:text-neutral-900"
            >
              {busy ? <Loader2 size={16} className="animate-spin" /> : <KeyRound size={16} />}
              Activate passkey for this device
            </button>

            <button
              type="button"
              onClick={() => {
                setMode('signin');
                setError(null);
              }}
              className="text-sm text-neutral-600 underline underline-offset-4 hover:text-neutral-900 dark:text-neutral-400 dark:hover:text-neutral-100"
            >
              Back to sign in
            </button>
          </div>
        ) : mode === 'signin' ? (
          <div className="flex flex-col gap-4">
            <p className="text-sm text-neutral-600 dark:text-neutral-400">
              Use the passkey on this device. There is nothing to type.
            </p>

            <button
              type="button"
              onClick={handleSignIn}
              disabled={busy || !supported}
              className="flex items-center justify-center gap-2 rounded-lg bg-neutral-900 px-4 py-2.5 font-medium text-white disabled:opacity-60 dark:bg-neutral-100 dark:text-neutral-900"
            >
              {busy ? <Loader2 size={16} className="animate-spin" /> : <KeyRound size={16} />}
              Sign in
            </button>

            <button
              type="button"
              onClick={() => {
                setMode('register');
                setError(null);
              }}
              className="text-sm text-neutral-600 underline underline-offset-4 hover:text-neutral-900 dark:text-neutral-400 dark:hover:text-neutral-100"
            >
              Create an account instead
            </button>

            <button
              type="button"
              onClick={() => {
                setMode('recovery');
                setError(null);
              }}
              className="text-xs text-neutral-500 underline underline-offset-4 hover:text-neutral-800 dark:hover:text-neutral-200"
            >
              Lost your device? Enter a recovery code
            </button>
          </div>
        ) : mode === 'recovery' ? (
          <div className="flex flex-col gap-4">
            <p className="text-sm text-neutral-600 dark:text-neutral-400">
              Enter one of your 10 backup recovery codes or a matured code from the Matrix <code className="font-mono text-xs">!recover</code> flow.
            </p>

            <label className="flex flex-col gap-1.5">
              <span className="text-sm font-medium text-neutral-700 dark:text-neutral-300">
                Recovery code
              </span>
              <input
                type="text"
                value={recoveryCode}
                onChange={(e) => setRecoveryCode(e.target.value)}
                placeholder="e.g. 1a2b-3c4d-5e6f"
                autoFocus
                className="font-mono rounded-lg border border-neutral-300 px-3 py-2 text-neutral-900 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
              />
            </label>

            <button
              type="button"
              onClick={handleRecover}
              disabled={busy}
              className="flex items-center justify-center gap-2 rounded-lg bg-neutral-900 px-4 py-2.5 font-medium text-white disabled:opacity-60 dark:bg-neutral-100 dark:text-neutral-900"
            >
              {busy ? <Loader2 size={16} className="animate-spin" /> : <KeyRound size={16} />}
              Redeem and regain access
            </button>

            <button
              type="button"
              onClick={() => {
                setMode('signin');
                setError(null);
              }}
              className="text-sm text-neutral-600 underline underline-offset-4 hover:text-neutral-900 dark:text-neutral-400 dark:hover:text-neutral-100"
            >
              Back to sign in
            </button>
          </div>
        ) : (
          <div className="flex flex-col gap-4">
            <label className="flex flex-col gap-1.5">
              <span className="text-sm font-medium text-neutral-700 dark:text-neutral-300">
                Display name
              </span>
              <input
                type="text"
                value={displayName}
                onChange={(e) => setDisplayName(e.target.value)}
                maxLength={64}
                autoFocus
                className="rounded-lg border border-neutral-300 px-3 py-2 text-neutral-900 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
              />
              <span className="text-xs text-neutral-500 dark:text-neutral-400">
                Shown on your account. Not used to sign in.
              </span>
            </label>

            <label className="flex flex-col gap-1.5">
              <span className="text-sm font-medium text-neutral-700 dark:text-neutral-300">
                Invite code
              </span>
              <input
                type="password"
                value={invite}
                onChange={(e) => setInvite(e.target.value)}
                className="rounded-lg border border-neutral-300 px-3 py-2 text-neutral-900 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
              />
              <span className="text-xs text-neutral-500 dark:text-neutral-400">
                Leave blank if this server does not require one.
              </span>
            </label>

            <button
              type="button"
              onClick={handleRegister}
              disabled={busy || !supported}
              className="flex items-center justify-center gap-2 rounded-lg bg-neutral-900 px-4 py-2.5 font-medium text-white disabled:opacity-60 dark:bg-neutral-100 dark:text-neutral-900"
            >
              {busy ? <Loader2 size={16} className="animate-spin" /> : <KeyRound size={16} />}
              Create passkey
            </button>

            <button
              type="button"
              onClick={() => {
                setMode('signin');
                setError(null);
              }}
              className="text-sm text-neutral-600 underline underline-offset-4 hover:text-neutral-900 dark:text-neutral-400 dark:hover:text-neutral-100"
            >
              I already have an account
            </button>
          </div>
        )}
      </div>
    </div>
  );
};
