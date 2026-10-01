import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { CaptureVersion, RefreshControlState } from './contracts.ts';
import { canRequestRefresh, mergeRefreshState, refreshProgressObserved } from './logic.ts';

interface RefreshRequest {
  version: CaptureVersion;
  settled: boolean;
  observed: boolean;
}

export function useRefreshController() {
  const native = isTauri();
  const [state, setState] = useState<RefreshControlState | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState('');
  const stateRef = useRef<RefreshControlState | null>(null);
  const requestRef = useRef<RefreshRequest | null>(null);
  const mountedRef = useRef(false);

  const releaseCompletedRequest = useCallback((request: RefreshRequest) => {
    if (mountedRef.current && requestRef.current === request && request.settled && request.observed) {
      requestRef.current = null;
      setPending(false);
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    let disposed = false;
    let unlisten: UnlistenFn | undefined;
    if (native) {
      void listen<RefreshControlState>('refresh-state', ({ payload }) => {
        if (disposed) return;
        const previous = stateRef.current;
        const accepted = mergeRefreshState(previous, payload);
        if (accepted !== previous) {
          stateRef.current = accepted;
          setState(accepted);
          if (previous !== null && (previous.session_id !== accepted.session_id || previous.round_epoch !== accepted.round_epoch)) {
            requestRef.current = null;
            setPending(false);
            setError('');
          }
        }
        const request = requestRef.current;
        if (request !== null && refreshProgressObserved(request.version, accepted)) {
          request.observed = true;
          releaseCompletedRequest(request);
        }
      }).then((off) => { if (disposed) off(); else unlisten = off; })
        .catch((cause) => { if (!disposed) setError(cause instanceof Error ? cause.message : String(cause)); });
    }
    return () => {
      disposed = true;
      mountedRef.current = false;
      requestRef.current = null;
      unlisten?.();
    };
  }, [native, releaseCompletedRequest]);

  const refresh = useCallback(async () => {
    const snapshot = stateRef.current;
    if (!canRequestRefresh(native, snapshot, requestRef.current !== null) || snapshot === null) return;
    const request: RefreshRequest = {
      version: { session_id: snapshot.session_id, round_epoch: snapshot.round_epoch, revision: snapshot.revision },
      settled: false,
      observed: false,
    };
    // The ref locks before React renders, so two clicks cannot invoke twice.
    requestRef.current = request;
    setPending(true);
    setError('');
    try {
      await invoke('refresh_capture', {
        expectedSession: snapshot.session_id,
        expectedRevision: snapshot.revision,
        expectedRoundEpoch: snapshot.round_epoch,
      });
      request.settled = true;
      releaseCompletedRequest(request);
    } catch (cause) {
      if (!mountedRef.current || requestRef.current !== request) return;
      requestRef.current = null;
      setPending(false);
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }, [native, releaseCompletedRequest]);

  return { native, state, pending, error, refresh, enabled: canRequestRefresh(native, state, pending) };
}
