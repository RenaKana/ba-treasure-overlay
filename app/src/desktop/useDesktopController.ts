import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  CaptureState,
  CaptureVersion,
  CellState,
  ItemSpec,
  ObservedCell,
  OverlayResult,
  Rect,
  SolverResponse,
  UpdateMode,
  WindowChoice,
} from './contracts.ts';
import {
  calculationKey,
  canStartCalculation,
  chooseWindow,
  filterMask,
  isCurrentResult,
  selectedProbabilities,
  selectedInferredPlacements,
  shouldAcceptCapture,
  validItems,
  validSolverResult,
} from './logic.ts';
import { armSolverTimeout } from './solverTimeout.ts';

const initialItems = (): ItemSpec[] =>
  Array.from({ length: 3 }, () => ({ width: 0, height: 0, remaining_count: -1 }));

type SolverPhase = 'waiting' | 'calculating' | 'ready' | 'error';
const EMPHASIZE_BEST_STORAGE_KEY = 'ba-emphasize-best';

function storedEmphasizeBest(): boolean {
  try {
    return localStorage.getItem(EMPHASIZE_BEST_STORAGE_KEY) === 'true';
  } catch {
    return false;
  }
}

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

function solverErrorMessage(message: string): string {
  if (message.startsWith('no_valid_configuration')) return '棋盘与物品设置不一致，请校正格子或参数';
  if (message.startsWith('input_error')) return '物品参数无效，请检查尺寸和数量';
  if (message.startsWith('configuration_count_overflow')) return '棋盘布局过多，暂时无法计算';
  return message;
}

function stateToken(state: CaptureState | null): string {
  return state === null
    ? ''
    : `${state.session_id}:${state.round_epoch}:${state.revision}:${state.status}:${state.confirmed}:${state.update_mode}:${state.refreshing}`;
}

export function useDesktopController() {
  const native = isTauri();
  const [listening, setListening] = useState(false);
  const [windows, setWindows] = useState<WindowChoice[]>([]);
  const [selectedWindow, setSelectedWindow] = useState('');
  const [capture, setCapture] = useState<CaptureState | null>(null);
  const [capturing, setCapturing] = useState(false);
  const [items, setItems] = useState<ItemSpec[]>(initialItems);
  const [dirty, setDirty] = useState(false);
  const [calibrating, setCalibrating] = useState(false);
  const [overlayVisible, setOverlayVisible] = useState(true);
  const [emphasizeBest, setEmphasizeBest] = useState(storedEmphasizeBest);
  const [filters, setFilters] = useState([true, true, true]);
  const [busy, setBusy] = useState('');
  const [awaitingChange, setAwaitingChange] = useState(false);
  const [error, setError] = useState('');
  const [solverPhase, setSolverPhase] = useState<SolverPhase>('waiting');
  const [solverMessage, setSolverMessage] = useState('');
  const [result, setResult] = useState<SolverResponse | null>(null);

  const captureRef = useRef<CaptureState | null>(null);
  const sessionRef = useRef<number | null>(null);
  const workerRef = useRef<Worker | null>(null);
  const manualJobRef = useRef<string | null>(null);
  const generationRef = useRef(0);
  const publishTokenRef = useRef(0);
  const publishQueueRef = useRef<Promise<void>>(Promise.resolve());
  const dirtyRef = useRef(false);
  const calibratingRef = useRef(false);
  const overlayVisibleRef = useRef(true);
  const emphasizeBestRef = useRef(emphasizeBest);
  const barrierRef = useRef<string | null>(null);
  const filtersRef = useRef(filters);
  const busyRef = useRef('');

  const reportSolverFailure = useCallback((version: CaptureVersion, message: string) => {
    if (!native || !isCurrentResult(version, captureRef.current)) return;
    const generation = generationRef.current;
    publishQueueRef.current = publishQueueRef.current.catch(() => undefined).then(async () => {
      if (generation !== generationRef.current || !isCurrentResult(version, captureRef.current)) return;
      await invoke('solver_failed', {
        sessionId: version.session_id,
        roundEpoch: version.round_epoch,
        revision: version.revision,
        message,
      });
    }).catch((cause) => {
      if (generation === generationRef.current && isCurrentResult(version, captureRef.current)) setError(errorMessage(cause));
    });
  }, [native]);

  const invalidate = useCallback(() => {
    generationRef.current += 1;
    publishTokenRef.current += 1;
    workerRef.current?.terminate();
    workerRef.current = null;
    setResult(null);
    setSolverPhase('waiting');
    setSolverMessage('');
    if (native) {
      const token = publishTokenRef.current;
      publishQueueRef.current = publishQueueRef.current.catch(() => undefined).then(async () => {
        if (token !== publishTokenRef.current) return;
        await invoke('clear_overlay');
      }).catch((cause) => {
        if (token === publishTokenRef.current) setError(errorMessage(cause));
      });
    }
  }, [native]);

  const refreshWindows = useCallback(async () => {
    if (!native) return;
    try {
      const choices = await invoke<WindowChoice[]>('list_windows');
      setWindows(choices);
      setSelectedWindow((previous) => chooseWindow(choices, previous));
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }, [native]);

  useEffect(() => {
    if (!native) return;
    let disposed = false;
    let unlisten: UnlistenFn | undefined;
    void listen<CaptureState>('capture-state', ({ payload }) => {
      if (disposed || !shouldAcceptCapture(captureRef.current, payload, sessionRef.current)) {
        return;
      }
      const previous = captureRef.current;
      const newSession = previous?.session_id !== payload.session_id;
      const newRound = previous?.round_epoch !== payload.round_epoch;
      if (newSession || newRound) {
        dirtyRef.current = false;
        calibratingRef.current = false;
        setDirty(false);
        setCalibrating(false);
      }
      if (
        newSession ||
        newRound ||
        previous?.revision !== payload.revision ||
        calculationKey(previous) !== calculationKey(payload)
      ) {
        invalidate();
      }
      if (barrierRef.current !== null && barrierRef.current !== stateToken(payload)) {
        barrierRef.current = null;
        setAwaitingChange(false);
      }
      captureRef.current = payload;
      setCapture(payload);
      if (!dirtyRef.current && payload.items.length === 3) {
        setItems(payload.items.map((item) => ({ ...item })));
      }
    }).then((off) => {
      if (disposed) off();
      else {
        unlisten = off;
        setListening(true);
        void refreshWindows();
      }
    }).catch((cause) => setError(errorMessage(cause)));
    return () => {
      disposed = true;
      unlisten?.();
      workerRef.current?.terminate();
      generationRef.current += 1;
      publishTokenRef.current += 1;
    };
  }, [native, invalidate, refreshWindows]);

  const publishResult = useCallback((response: SolverResponse) => {
    const state = captureRef.current;
    const probabilities = selectedProbabilities(response.probs, filterMask(filtersRef.current));
    if (
      !native ||
      !isCurrentResult(response, state) ||
      state === null ||
      probabilities === null ||
      response.precision === null ||
      dirtyRef.current ||
      calibratingRef.current ||
      barrierRef.current !== null
    ) return;

    const token = ++publishTokenRef.current;
    const generation = generationRef.current;
    const result: OverlayResult = {
      session_id: response.session_id,
      round_epoch: response.round_epoch,
      revision: response.revision,
      probabilities,
      cells: [...state.cells],
      precision: response.precision,
      message: response.precision === 'exact' ? '精确概率' : '估计概率',
      inferred_placements: selectedInferredPlacements(response, filterMask(filtersRef.current)),
      emphasize_best: emphasizeBestRef.current,
    };
    // Serial publication prevents a quickly changed filter from being overwritten.
    publishQueueRef.current = publishQueueRef.current.catch(() => undefined).then(async () => {
      if (token !== publishTokenRef.current || generation !== generationRef.current) return;
      await invoke('render_overlay', { result });
      if (
        token !== publishTokenRef.current ||
        generation !== generationRef.current ||
        !isCurrentResult(response, captureRef.current) ||
        dirtyRef.current ||
        calibratingRef.current ||
        barrierRef.current !== null
      ) return;
      await invoke('set_overlay_visible', { visible: overlayVisibleRef.current });
    }).catch((cause) => {
      const message = errorMessage(cause);
      if (token === publishTokenRef.current && isCurrentResult(response, captureRef.current)) {
        setError(message);
        reportSolverFailure(response, message);
      }
    });
  }, [native, reportSolverFailure]);

  const jobKey = !dirty && !calibrating && !awaitingChange && capturing
    ? calculationKey(capture)
    : null;

  useEffect(() => {
    if (!native || jobKey === null) return;
    const state = captureRef.current;
    if (state === null || calculationKey(state) !== jobKey || !canStartCalculation(state, manualJobRef.current)) return;
    const generation = ++generationRef.current;
    if (state.update_mode === 'manual') manualJobRef.current = jobKey;
    setSolverPhase('calculating');
    setSolverMessage('正在计算当前棋盘');
    const fail = (message: string) => {
      if (generation !== generationRef.current || !isCurrentResult(state, captureRef.current)) return;
      setResult(null);
      setSolverPhase('error');
      setSolverMessage(message);
      reportSolverFailure(state, message);
    };
    let worker: Worker;
    try {
      worker = new Worker(new URL('./solver.worker.ts', import.meta.url), { type: 'module' });
    } catch (cause) {
      fail(errorMessage(cause));
      return;
    }
    workerRef.current = worker;
    const currentWorker = () => generation === generationRef.current && workerRef.current === worker &&
      isCurrentResult(state, captureRef.current) && !dirtyRef.current && !calibratingRef.current && barrierRef.current === null;
    const cancelTimeout = armSolverTimeout(
      currentWorker,
      () => {
        worker.terminate();
        workerRef.current = null;
        fail('计算超时，请点击刷新重试');
      },
    );
    worker.onmessage = (event: MessageEvent<SolverResponse>) => {
      const response = event.data;
      if (
        generation !== generationRef.current ||
        workerRef.current !== worker ||
        !isCurrentResult(response, captureRef.current) ||
        dirtyRef.current ||
        calibratingRef.current ||
        barrierRef.current !== null
      ) return;
      cancelTimeout();
      worker.terminate();
      workerRef.current = null;
      if (!validSolverResult(response)) {
        fail(solverErrorMessage(response.error) || '计算结果不完整，请检查棋盘和物品');
        return;
      }
      setResult(response);
      setSolverPhase('ready');
      setSolverMessage(response.precision === 'exact' ? '精确概率' : '估计概率');
      publishResult(response);
    };
    worker.onerror = (event) => {
      if (workerRef.current !== worker) return;
      cancelTimeout();
      worker.terminate();
      workerRef.current = null;
      fail(event.message || '计算程序未能启动');
    };
    publishQueueRef.current = publishQueueRef.current.catch(() => undefined).then(async () => {
      if (!currentWorker()) return;
      await invoke('solver_started', { sessionId: state.session_id, roundEpoch: state.round_epoch, revision: state.revision });
      if (!currentWorker()) return;
      worker.postMessage({
        session_id: state.session_id,
        round_epoch: state.round_epoch,
        revision: state.revision,
        input: { items: state.items, cells: state.cells as ObservedCell[], candidate_constraints: state.candidate_constraints },
      });
    }).catch((cause) => {
      if (!currentWorker()) return;
      cancelTimeout();
      worker.terminate();
      workerRef.current = null;
      fail(errorMessage(cause));
    });
    return () => {
      cancelTimeout();
      worker.terminate();
      if (workerRef.current === worker) workerRef.current = null;
      generationRef.current += 1;
    };
  }, [native, jobKey, publishResult, reportSolverFailure]);

  async function mutate(command: string, args?: Record<string, unknown>): Promise<boolean> {
    if (!native || busyRef.current !== '') return false;
    const snapshot = captureRef.current;
    let commandArgs = args;
    if (
      command === 'confirm_items' || command === 'calibrate' || command === 'correct_cell' ||
      command === 'set_update_mode' || command === 'refresh_capture'
    ) {
      if (snapshot === null) {
        setError('尚无捕获画面，请连接后重试');
        return false;
      }
      commandArgs = {
        ...args,
        expectedSession: snapshot.session_id,
        expectedRevision: snapshot.revision,
        expectedRoundEpoch: snapshot.round_epoch,
      };
    }
    if (command === 'refresh_capture' && snapshot?.refreshing) return false;
    barrierRef.current = stateToken(snapshot);
    setAwaitingChange(true);
    invalidate();
    busyRef.current = command;
    setBusy(command);
    setError('');
    try {
      await invoke(command, commandArgs);
      return true;
    } catch (cause) {
      barrierRef.current = null;
      setAwaitingChange(false);
      const message = errorMessage(cause);
      setError(command === 'calibrate' || command === 'correct_cell'
        ? `操作未保存，请重试：${message}`
        : message);
      return false;
    } finally {
      busyRef.current = '';
      setBusy('');
    }
  }

  async function startCapture() {
    if (!native || selectedWindow === '' || busyRef.current !== '') return;
    invalidate();
    captureRef.current = null;
    manualJobRef.current = null;
    sessionRef.current = null;
    barrierRef.current = null;
    dirtyRef.current = false;
    calibratingRef.current = false;
    setCapture(null);
    setDirty(false);
    setCalibrating(false);
    setAwaitingChange(false);
    setItems(initialItems());
    busyRef.current = 'start_capture';
    setBusy('start_capture');
    setError('');
    try {
      const session = await invoke<number>('start_capture', { hwnd: Number(selectedWindow) });
      sessionRef.current = session;
      // Capture events may arrive while invoke is awaited.
      const arrived = captureRef.current as CaptureState | null;
      if (arrived !== null && arrived.session_id !== session) {
        captureRef.current = null;
        setCapture(null);
      }
      setCapturing(true);
    } catch (cause) {
      setCapturing(false);
      setError(errorMessage(cause));
    } finally {
      busyRef.current = '';
      setBusy('');
    }
  }

  async function stopCapture() {
    if (!native || busyRef.current !== '') return;
    invalidate();
    busyRef.current = 'stop_capture';
    setBusy('stop_capture');
    try {
      await invoke('stop_capture');
      captureRef.current = null;
      sessionRef.current = null;
      barrierRef.current = null;
      dirtyRef.current = false;
      calibratingRef.current = false;
      setCapture(null);
      setCapturing(false);
      setDirty(false);
      setCalibrating(false);
      setAwaitingChange(false);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      busyRef.current = '';
      setBusy('');
    }
  }

  function editItem(index: number, field: keyof ItemSpec, value: number) {
    dirtyRef.current = true;
    setDirty(true);
    invalidate();
    setItems((previous) => previous.map((item, itemIndex) =>
      itemIndex === index ? { ...item, [field]: value } : item,
    ));
  }

  async function confirmItems() {
    if (!validItems(items)) return;
    dirtyRef.current = false;
    setDirty(false);
    if (!(await mutate('confirm_items', { items }))) {
      dirtyRef.current = true;
      setDirty(true);
    }
  }

  function toggleCalibration() {
    const next = !calibratingRef.current;
    calibratingRef.current = next;
    setCalibrating(next);
    invalidate();
  }

  async function calibrate(rect: Rect) {
    calibratingRef.current = false;
    setCalibrating(false);
    await mutate('calibrate', { rect });
  }

  async function correctCell(index: number, cell: CellState) {
    if (captureRef.current?.cells[index] === cell) return;
    await mutate('correct_cell', { index, cell });
  }

  async function resetRound() {
    dirtyRef.current = false;
    calibratingRef.current = false;
    setDirty(false);
    setCalibrating(false);
    await mutate('reset_round');
  }

  async function setUpdateMode(mode: UpdateMode) {
    if (captureRef.current?.update_mode === mode) return;
    await mutate('set_update_mode', { mode });
  }

  async function refreshCapture() {
    await mutate('refresh_capture');
  }

  function toggleFilter(index: number) {
    const next = filtersRef.current.map((enabled, itemIndex) => itemIndex === index ? !enabled : enabled);
    if (filterMask(next) === 0) return;
    filtersRef.current = next;
    setFilters(next);
    if (result !== null) publishResult(result);
  }

  function toggleEmphasizeBest() {
    const next = !emphasizeBestRef.current;
    emphasizeBestRef.current = next;
    setEmphasizeBest(next);
    try {
      localStorage.setItem(EMPHASIZE_BEST_STORAGE_KEY, String(next));
    } catch {
      // The preference still works for this session when storage is unavailable.
    }
    if (result !== null) publishResult(result);
  }

  async function toggleOverlay() {
    const visible = !overlayVisibleRef.current;
    overlayVisibleRef.current = visible;
    setOverlayVisible(visible);
    if (!native) return;
    // Preserve the user's display intent after any queued invalidation clears old probabilities.
    publishQueueRef.current = publishQueueRef.current.catch(() => undefined).then(async () => {
      if (overlayVisibleRef.current !== visible) return;
      await invoke('set_overlay_visible', { visible });
    }).catch((cause) => {
      if (overlayVisibleRef.current === visible) setError(errorMessage(cause));
    });
    await publishQueueRef.current;
  }

  const probabilities = result !== null && isCurrentResult(result, capture) &&
    !dirty && !calibrating && !awaitingChange
    ? selectedProbabilities(result.probs, filterMask(filters))
    : null;
  const inferredPlacements = probabilities !== null && result !== null
    ? selectedInferredPlacements(result, filterMask(filters))
    : [];

  return {
    native, listening, windows, selectedWindow, setSelectedWindow, refreshWindows,
    capture, capturing, items, dirty, editItem, confirmItems, calibrating,
    toggleCalibration, calibrate, correctCell, resetRound, overlayVisible,
    toggleOverlay, filters, toggleFilter, busy, awaitingChange, error,
    emphasizeBest, toggleEmphasizeBest,
    clearError: () => setError(''), solverPhase, solverMessage, result, probabilities, inferredPlacements,
    startCapture, stopCapture, setUpdateMode, refreshCapture,
    setPaused: (paused: boolean) => mutate('set_paused', { paused }),
  };
}
