import React, { useState, useEffect } from 'react';
import { fetchApi, setAuthToken, clearAuthToken, getAuthToken } from '../core/apiClient';
import { X, Lock, User, LogOut, Copy, Check } from 'lucide-react';

interface AuthModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSuccess: (token: string | null) => void;
  userToken?: string | null;
}

export const AuthModal: React.FC<AuthModalProps> = ({
  isOpen,
  onClose,
  onSuccess,
  userToken,
}) => {
  const [mode, setMode] = useState<'login' | 'register'>('login');
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [secret, setSecret] = useState('');
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && isOpen) {
        onClose();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  const currentToken = userToken || getAuthToken();

  const handleLogout = () => {
    clearAuthToken();
    onSuccess(null);
    onClose();
  };

  const handleCopyToken = () => {
    if (currentToken) {
      navigator.clipboard.writeText(currentToken);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setErrorMsg(null);
    setLoading(true);

    try {
      if (mode === 'login') {
        const res = await fetchApi<{ token: string; username: string }>('/auth/login', {
          method: 'POST',
          body: JSON.stringify({ username, password }),
        });
        setAuthToken(res.token);
        onSuccess(res.token);
        onClose();
      } else {
        const res = await fetchApi<{ token: string; username: string }>('/auth/register', {
          method: 'POST',
          body: JSON.stringify({ username, password, secret: secret || undefined }),
        });
        setAuthToken(res.token);
        onSuccess(res.token);
        onClose();
      }
    } catch (err: unknown) {
      setErrorMsg(err instanceof Error ? err.message : 'Authentication failed');
    } finally {
      setLoading(false);
    }
  };

  const maskedToken = currentToken
    ? `${currentToken.substring(0, 12)}...${currentToken.substring(currentToken.length - 8)}`
    : '';

  return (
    <div
      className="fixed inset-0 z-50 bg-black/60 dark:bg-black/80 backdrop-blur-sm flex items-center justify-center p-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="auth-modal-title"
    >
      <div className="bg-white dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 rounded-lg max-w-md w-full p-6 space-y-6 shadow-2xl">
        <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-850 pb-4">
          <div className="flex items-center gap-2">
            {currentToken ? (
              <User className="w-4 h-4 text-emerald-500 dark:text-emerald-400" />
            ) : (
              <Lock className="w-4 h-4 text-neutral-700 dark:text-neutral-300" />
            )}
            <h3 id="auth-modal-title" className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
              {currentToken
                ? 'Session Information'
                : mode === 'login'
                ? 'Authentication'
                : 'Register Web Account'}
            </h3>
          </div>
          <button
            onClick={onClose}
            aria-label="Close authentication modal"
            className="text-neutral-400 hover:text-neutral-900 dark:hover:text-white transition"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {errorMsg && (
          <div className="p-3 rounded bg-rose-100 dark:bg-rose-950/40 border border-rose-200 dark:border-rose-800/50 text-rose-800 dark:text-rose-300 text-xs font-mono">
            {errorMsg}
          </div>
        )}

        {currentToken ? (
          /* Authenticated User View */
          <div className="space-y-5">
            <div className="flex items-center justify-between p-3 rounded bg-emerald-50 dark:bg-emerald-950/30 border border-emerald-200 dark:border-emerald-900/40">
              <div className="flex items-center gap-2">
                <span className="w-2 h-2 rounded-full bg-emerald-500 animate-pulse" />
                <span className="text-xs font-mono font-medium text-emerald-900 dark:text-emerald-300">
                  Active Web Session
                </span>
              </div>
              <span className="text-[10px] font-mono text-emerald-700 dark:text-emerald-400">JWT Token Valid</span>
            </div>

            <div className="space-y-1.5">
              <div className="flex items-center justify-between">
                <label className="text-xs font-mono text-neutral-600 dark:text-neutral-400">Authorization Token</label>
                <button
                  type="button"
                  onClick={handleCopyToken}
                  className="flex items-center gap-1 text-[11px] font-mono text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200 transition"
                >
                  {copied ? (
                    <>
                      <Check className="w-3 h-3 text-emerald-500" />
                      <span className="text-emerald-500">Copied!</span>
                    </>
                  ) : (
                    <>
                      <Copy className="w-3 h-3" />
                      <span>Copy Bearer Token</span>
                    </>
                  )}
                </button>
              </div>

              <div className="p-3 rounded bg-neutral-100 dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 font-mono text-xs text-neutral-800 dark:text-neutral-300 break-all select-none">
                {maskedToken}
              </div>
            </div>

            <div className="pt-2 flex items-center justify-between">
              <button
                type="button"
                onClick={() => {
                  clearAuthToken();
                  setMode('login');
                  onSuccess(null);
                }}
                className="text-xs font-mono text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200 transition underline"
              >
                Switch Account
              </button>

              <button
                type="button"
                onClick={handleLogout}
                className="inline-flex items-center gap-2 px-4 py-2 rounded bg-rose-600 dark:bg-rose-600 text-white text-xs font-medium hover:bg-rose-700 transition"
              >
                <LogOut className="w-3.5 h-3.5" />
                <span>Logout</span>
              </button>
            </div>
          </div>
        ) : (
          /* Login / Register Form View */
          <form onSubmit={handleSubmit} className="space-y-4">
            <div className="space-y-1.5">
              <label className="text-xs font-mono text-neutral-600 dark:text-neutral-400">Username</label>
              <input
                type="text"
                required
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                placeholder="e.g. alice"
                className="w-full px-3 py-2 rounded bg-neutral-50 dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-600 focus:outline-none focus:border-neutral-500 font-sans"
              />
            </div>

            <div className="space-y-1.5">
              <label className="text-xs font-mono text-neutral-600 dark:text-neutral-400">Password</label>
              <input
                type="password"
                required
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder="••••••••"
                className="w-full px-3 py-2 rounded bg-neutral-50 dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-600 focus:outline-none focus:border-neutral-500 font-sans"
              />
            </div>

            {mode === 'register' && (
              <div className="space-y-1.5">
                <label className="text-xs font-mono text-neutral-600 dark:text-neutral-400">Registration Secret (Optional)</label>
                <input
                  type="password"
                  value={secret}
                  onChange={(e) => setSecret(e.target.value)}
                  placeholder="Optional registration secret"
                  className="w-full px-3 py-2 rounded bg-neutral-50 dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-600 focus:outline-none focus:border-neutral-500 font-sans"
                />
              </div>
            )}

            <div className="pt-2 flex items-center justify-between">
              <button
                type="button"
                onClick={() => setMode(mode === 'login' ? 'register' : 'login')}
                className="text-xs font-mono text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200 transition underline"
              >
                {mode === 'login' ? 'Need an account? Register' : 'Have an account? Login'}
              </button>

              <button
                type="submit"
                disabled={loading}
                className="px-4 py-2 rounded bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-white transition disabled:opacity-50"
              >
                {loading ? 'Authenticating...' : mode === 'login' ? 'Login' : 'Register'}
              </button>
            </div>
          </form>
        )}
      </div>
    </div>
  );
};
