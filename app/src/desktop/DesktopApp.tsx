import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type PointerEvent } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import {
  BOARD_COLUMNS,
  BOARD_ROWS,
  BOARD_SIZE,
  type CaptureState,
  type CellState,
  type OverlayState,
  type Rect,
} from './contracts.ts';
import { bestCells, mergeOverlayState, percent, selectionRect, validItems } from './logic.ts';
import { useDesktopController } from './useDesktopController.ts';
import { useRefreshController } from './useRefreshController.ts';
import { InferenceLayer, OverlayView } from './ProbabilityView.tsx';
import DesktopZoom from './DesktopZoom.tsx';
import CoverReferencePicker from './CoverReferencePicker.tsx';
import { DESKTOP_LOCALES, translateDesktopMessage, useDesktopI18n, type DesktopLocale } from './desktopI18n.ts';
import './desktop.css';

const cellLabels: Record<CellState, string> = {
  unknown: '未翻', empty: '空格', item0: '物品 1', item1: '物品 2',
  item2: '物品 3', completed: '已完成', uncertain: '待确认',
};
const cellSymbols: Record<CellState, string> = {
  unknown: '·', empty: '空', item0: '1', item1: '2', item2: '3', completed: '✓', uncertain: '!',
};
const statusLabels = {
  manual: '手动快照',
  searching: '寻找棋盘', ready: '已识别棋盘', uncertain: '需要校正', waiting_item: '等待物品翻完',
  paused: '已暂停', away: '等待游戏窗口', error: '连接异常',
};

function Icon({ name }: { name: 'refresh' | 'connect' | 'stop' | 'pause' | 'play' | 'eye' | 'hidden' | 'crop' | 'reset' | 'check' | 'close' }) {
  const paths = {
    refresh: 'M18 7a7 7 0 1 0 1 8M18 3v5h-5',
    connect: 'M8 3v5m8-5v5M6 8h12v3a6 6 0 0 1-12 0V8Zm6 9v4',
    stop: 'M6 6h12v12H6z',
    pause: 'M8 5v14M16 5v14',
    play: 'M8 4l12 8-12 8V4Z',
    eye: 'M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12ZM12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6',
    hidden: 'm3 3 18 18M10 5a13 13 0 0 1 12 7 19 19 0 0 1-4 5M6 6a19 19 0 0 0-4 6s4 7 10 7a13 13 0 0 0 4-1',
    crop: 'M7 2v15h15M2 7h15v15',
    reset: 'M4 10a8 8 0 1 1 1 7M4 4v6h6',
    check: 'm5 12 4 4L19 6',
    close: 'm6 6 12 12M18 6 6 18',
  };
  return <svg className="ba-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d={paths[name]} stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" /></svg>;
}

function useSurface(surface: 'control' | 'overlay' | 'refresh') {
  useLayoutEffect(() => {
    document.documentElement.dataset.baSurface = surface;
    return () => { delete document.documentElement.dataset.baSurface; };
  }, [surface]);
}

function rectStyle(rect: Rect): CSSProperties {
  return {
    left: `${rect.x * 100}%`, top: `${rect.y * 100}%`,
    width: `${rect.width * 100}%`, height: `${rect.height * 100}%`,
  };
}

function FramePreview({ capture, calibrating, onCalibrate, onCancel }: {
  capture: CaptureState | null;
  calibrating: boolean;
  onCalibrate: (rect: Rect) => void;
  onCancel: () => void;
}) {
  const { t } = useDesktopI18n();
  const stageRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<{ x: number; y: number } | null>(null);
  const [selection, setSelection] = useState<Rect | null>(null);
  const [displaySize, setDisplaySize] = useState({ width: 0, height: 0 });
  const hasFrame = Boolean(capture?.frame_url && capture.width > 0 && capture.height > 0);

  useLayoutEffect(() => {
    const stage = stageRef.current;
    const width = capture?.width ?? 0;
    const height = capture?.height ?? 0;
    if (stage === null || width <= 0 || height <= 0) return;
    const update = () => {
      const scale = Math.min(stage.clientWidth / width, stage.clientHeight / height);
      setDisplaySize({ width: width * scale, height: height * scale });
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(stage);
    return () => observer.disconnect();
  }, [capture?.width, capture?.height]);

  useEffect(() => {
    if (!calibrating) {
      dragRef.current = null;
      setSelection(null);
    }
  }, [calibrating]);

  function point(event: PointerEvent<HTMLDivElement>) {
    const bounds = contentRef.current?.getBoundingClientRect();
    if (bounds === undefined || bounds.width === 0 || bounds.height === 0) return null;
    return {
      x: Math.max(0, Math.min(1, (event.clientX - bounds.left) / bounds.width)),
      y: Math.max(0, Math.min(1, (event.clientY - bounds.top) / bounds.height)),
    };
  }

  return <div className={`ba-preview${calibrating ? ' is-calibrating' : ''}`} ref={stageRef}>
    {hasFrame && capture !== null ? <div
      className="ba-frame-content"
      ref={contentRef}
      style={displaySize.width > 0 ? { width: displaySize.width, height: displaySize.height } : undefined}
      tabIndex={calibrating ? 0 : -1}
      aria-label={t(calibrating ? '拖动框选棋盘，按 Escape 取消' : '游戏画面预览')}
      onKeyDown={(event) => { if (event.key === 'Escape' && calibrating) onCancel(); }}
      onPointerDown={(event) => {
        if (!calibrating || event.button !== 0) return;
        dragRef.current = point(event);
        setSelection(null);
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onPointerMove={(event) => {
        const end = point(event);
        if (dragRef.current !== null && end !== null) setSelection(selectionRect(dragRef.current, end));
      }}
      onPointerUp={(event) => {
        const end = point(event);
        const rect = dragRef.current !== null && end !== null ? selectionRect(dragRef.current, end) : null;
        dragRef.current = null;
        if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
        if (rect !== null) onCalibrate(rect);
      }}
      onPointerCancel={() => { dragRef.current = null; setSelection(null); }}
    >
      <img className="ba-frame-image" src={capture.frame_url} alt={t('当前游戏窗口画面')} draggable={false} />
      {(selection ?? capture.board) !== null && <div className={`ba-frame-board${selection !== null ? ' is-selection' : ''}`} style={rectStyle((selection ?? capture.board)!)}>
        <div className="ba-frame-grid">{Array.from({ length: BOARD_SIZE }, (_, index) => <span key={index} />)}</div>
        <span className="ba-frame-board-label">9 × 5</span>
      </div>}
      {calibrating && <span className="ba-preview-hint">{t('拖出棋盘边界 · Esc 取消')}</span>}
    </div> : <div className="ba-preview-empty">
      <div className="ba-empty-grid" aria-hidden="true">{Array.from({ length: BOARD_SIZE }, (_, index) => <i key={index} />)}</div>
      <span className="ba-empty-focus" aria-hidden="true" />
      <strong>{t(capture === null ? '等待游戏画面' : capture.refreshing ? '正在刷新游戏画面' : capture.update_mode === 'manual' ? '等待手动刷新' : '正在获取游戏画面')}</strong>
      <span>{t(capture === null ? '连接窗口后显示预览' : capture.refreshing ? '正在读取稳定画面' : capture.update_mode === 'manual' ? '点击上方刷新获取画面' : '保持游戏窗口可见')}</span>
    </div>}
  </div>;
}

function TransparentOverlay() {
  useSurface('overlay');
  const [overlay, setOverlay] = useState<OverlayState>({
    session_id: 0, round_epoch: 0, revision: 0,
    probabilities: [], cells: [], precision: '', message: '', visible: false,
    inferred_placements: [], emphasize_best: false,
  });
  const paintedVersionsRef = useRef({ session_id: 0, round_epoch: 0, revisions: new Set<number>() });
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false;
    let unlisten: UnlistenFn | undefined;
    void listen<OverlayState>('overlay-state', ({ payload }) => {
      if (disposed) return;
      // Repeated watchdog events retain the object and both pending paint frames.
      setOverlay((previous) => mergeOverlayState(previous, payload));
    }).then((off) => { if (disposed) off(); else unlisten = off; });
    return () => { disposed = true; unlisten?.(); };
  }, []);

  const complete = overlay.probabilities.length === BOARD_SIZE && overlay.cells.length === BOARD_SIZE;
  useEffect(() => {
    if (paintedVersionsRef.current.session_id !== overlay.session_id || paintedVersionsRef.current.round_epoch !== overlay.round_epoch) {
      paintedVersionsRef.current = { session_id: overlay.session_id, round_epoch: overlay.round_epoch, revisions: new Set() };
    }
    if (!isTauri() || !overlay.visible || !complete || paintedVersionsRef.current.revisions.has(overlay.revision)) return;
    let cancelled = false;
    let secondFrame = 0;
    const firstFrame = requestAnimationFrame(() => {
      secondFrame = requestAnimationFrame(() => {
        if (cancelled) return;
        paintedVersionsRef.current.revisions.add(overlay.revision);
        void invoke('overlay_painted', { sessionId: overlay.session_id, roundEpoch: overlay.round_epoch, revision: overlay.revision })
          .catch((cause) => console.warn('概率层绘制回执失败', cause));
      });
    });
    return () => {
      cancelled = true;
      cancelAnimationFrame(firstFrame);
      if (secondFrame !== 0) cancelAnimationFrame(secondFrame);
    };
  }, [overlay, complete]);
  return <OverlayView overlay={overlay} />;
}

function RefreshControl() {
  useSurface('refresh');
  const { t } = useDesktopI18n();
  const { native, state, pending, error, refresh, enabled } = useRefreshController();
  const refreshing = pending || state?.refreshing === true;
  const attention = error !== '' || state?.attention === true;
  const message = error !== '' ? error : state?.message || (native ? '等待游戏状态' : '桌面程序未连接');
  return <main className="ba-refresh-surface">
    <button
      className={`ba-game-refresh${refreshing ? ' is-refreshing' : ''}${attention ? ' needs-attention' : ''}`}
      type="button"
      aria-label={t('刷新棋盘概率')}
      aria-busy={refreshing}
      title={`${t('刷新助手识别与概率（不会切换游戏轮次）')}\n${translateDesktopMessage(state?.message || '', t)}${error !== '' ? `\n${translateDesktopMessage(error, t)}` : state?.message ? '' : translateDesktopMessage(message, t)}`}
      disabled={!enabled}
      onClick={() => { void refresh(); }}
    >
      <Icon name="refresh" />
      {attention && !refreshing && <span className="ba-refresh-attention" aria-hidden="true">!</span>}
    </button>
  </main>;
}

function ControlWindow() {
  useSurface('control');
  const { t, locale, setLocale } = useDesktopI18n();
  const controller = useDesktopController();
  const {
    native, listening, windows, selectedWindow, setSelectedWindow, refreshWindows,
    capture, capturing, items, dirty, editItem, confirmItems, calibrating,
    toggleCalibration, calibrate, correctCell, resetRound, overlayVisible,
    toggleOverlay, filters, toggleFilter, busy, awaitingChange, error, clearError,
    solverPhase, solverMessage, result, probabilities, startCapture, stopCapture, setPaused,
    setUpdateMode, refreshCapture, inferredPlacements, emphasizeBest, toggleEmphasizeBest,
    coverReference, coverSelection, beginCoverSelection, saveCoverSelection, cancelCoverSelection, clearCoverReference,
  } = controller;
  const [brush, setBrush] = useState<CellState>('empty');
  const cells = capture?.cells.length === BOARD_SIZE ? capture.cells : Array<CellState>(BOARD_SIZE).fill('unknown');
  const best = probabilities === null ? new Set<number>() : bestCells(probabilities, cells);
  const paused = capture?.status === 'paused';
  const manual = (capture?.update_mode ?? 'manual') === 'manual';
  const refreshing = capture?.refreshing === true;
  const editingDisabled = busy !== '' || awaitingChange || refreshing || coverSelection !== null;
  const modeDisabled = !native || !capturing || capture === null || editingDisabled || calibrating;
  const confirmed = Boolean(capture?.confirmed && !dirty && validItems(items));
  const uncertainCount = cells.filter((cell) => cell === 'uncertain').length;
  const status = !native ? '桌面程序未连接' : !capturing ? '未连接游戏' : capture === null ? '正在连接' : refreshing ? capture.status === 'ready' ? '正在计算' : '正在刷新' : manual && capture.status === 'ready' ? '手动快照' : statusLabels[capture.status];
  const statusKind = !native || !capturing ? 'idle' : refreshing ? 'searching' : manual && capture?.status === 'ready' ? 'manual' : capture?.status ?? 'searching';
  const capturedAt = capture?.captured_at_ms;
  const refreshedTime = typeof capturedAt === 'number' && Number.isFinite(capturedAt) && capturedAt > 0
    ? t('上次刷新 {{time}}', { time: new Date(capturedAt).toLocaleTimeString(locale, { hour12: false }) })
    : t('未刷新');
  let calculationStatus = solverMessage;
  if (!native) calculationStatus = '在桌面程序中连接游戏窗口';
  else if (!capturing) calculationStatus = '选择游戏窗口并连接';
  else if (refreshing && capture?.status !== 'ready') calculationStatus = '正在读取棋盘与物品卡片';
  else if (capture === null) calculationStatus = '等待游戏窗口';
  else if (calibrating) calculationStatus = manual ? '框选后点击刷新更新' : '框选棋盘后继续计算';
  else if (awaitingChange) calculationStatus = '等待状态更新';
  else if (dirty) calculationStatus = manual ? '校正参数后点击刷新' : '校正参数后计算';
  else if (!capture.confirmed || !validItems(items)) calculationStatus = '等待物品卡片；读取失败时可校正参数';
  else if (capture.status === 'manual') calculationStatus = '手动快照，点击刷新更新';
  else if (capture.status !== 'ready') calculationStatus = capture.message || statusLabels[capture.status];
  else if (solverPhase === 'waiting') calculationStatus = uncertainCount > 0 ? manual ? '校正待确认格子后点击刷新' : '校正待确认格子后计算' : manual ? '点击刷新更新手动快照' : '等待棋盘';

  return <main className="ba-control">
    <header className="ba-header">
      <div className="ba-brand-mark" aria-hidden="true"><span /><span /><span /><span /></div>
      <div className="ba-brand"><span className="ba-brand-kicker">{t('SCHALE / 寻宝')}</span><h1>{t('寻宝助手')}</h1></div>
      <div className="ba-display-options">
        <label htmlFor="desktop-language">{t('语言')}</label>
        <select id="desktop-language" aria-label={t('界面语言')} value={locale} onChange={(event) => setLocale(event.target.value as DesktopLocale)}>
          {DESKTOP_LOCALES.map(({ code, label }) => <option key={code} value={code}>{label}</option>)}
        </select>
        <DesktopZoom />
      </div>
      <span className={`ba-status ba-status-${statusKind}`}><i />{translateDesktopMessage(status, t)}</span>
    </header>

    <section className="ba-connection" aria-label={t('游戏窗口连接')}>
      <label className="ba-window-label" htmlFor="game-window">{t('游戏窗口')}</label>
      <select id="game-window" value={selectedWindow} onChange={(event) => setSelectedWindow(event.target.value)} disabled={!native || capturing || busy !== ''}>
        <option value="">{t(!native ? '桌面程序未连接' : windows.length === 0 ? '暂无可连接窗口' : '请选择游戏窗口')}</option>
        {windows.map((window) => <option key={window.hwnd} value={window.hwnd}>{window.title}</option>)}
      </select>
      <button className="ba-button ba-icon-button" type="button" title={t('刷新窗口')} aria-label={t('刷新窗口')} disabled={!native || capturing || busy !== ''} onClick={() => { void refreshWindows(); }}><Icon name="refresh" /></button>
      <button className={`ba-button${capturing ? '' : ' ba-button-primary'}`} type="button" disabled={!native || !listening || busy !== '' || (!capturing && selectedWindow === '')} onClick={() => { void (capturing ? stopCapture() : startCapture()); }}><Icon name={capturing ? 'stop' : 'connect'} />{t(busy === 'start_capture' ? '连接中' : capturing ? '停止' : '连接')}</button>
    </section>

    {error !== '' && <div className="ba-error" role="alert"><span>{translateDesktopMessage(error, t)}</span><button className="ba-button ba-icon-button" type="button" aria-label={t('关闭错误提示')} onClick={clearError}><Icon name="close" /></button></div>}

    <section className="ba-panel ba-capture-panel" aria-labelledby="capture-title">
      <div className="ba-panel-heading"><h2 id="capture-title">{t('游戏画面')}</h2><div className="ba-inline-actions">
        <div className="ba-mode-switch" role="group" aria-label={t('更新模式')}>
          <button className={`ba-mode-button${manual ? ' is-selected' : ''}`} type="button" aria-pressed={manual} title={t('仅在点击刷新时更新')} disabled={modeDisabled} onClick={() => { void setUpdateMode('manual'); }}>{t('手动')}</button>
          <button className={`ba-mode-button${!manual ? ' is-selected' : ''}`} type="button" aria-pressed={!manual} title={t('自动识别并更新棋盘')} disabled={modeDisabled} onClick={() => { void setUpdateMode('auto'); }}>{t('自动')}</button>
        </div>
        {manual && <button className="ba-button ba-button-small ba-icon-button ba-refresh-button" type="button" title={t(refreshing ? '正在刷新快照' : '刷新手动快照')} aria-label={t('刷新手动快照')} disabled={modeDisabled} onClick={() => { void refreshCapture(); }}><Icon name="refresh" /></button>}
        <button className={`ba-button ba-button-small${calibrating ? ' is-active' : ''}`} type="button" disabled={!native || !capture?.frame_url || editingDisabled} onClick={toggleCalibration}><Icon name="crop" />{t(calibrating ? '取消框选' : '框选棋盘')}</button>
        {!manual && <button className="ba-button ba-button-small" type="button" disabled={!native || !capturing || editingDisabled} onClick={() => { void setPaused(!paused); }}><Icon name={paused ? 'play' : 'pause'} />{t(paused ? '继续' : '暂停')}</button>}
        <button className="ba-button ba-button-small" type="button" disabled={!native || !capturing} onClick={() => { void toggleOverlay(); }}><Icon name={overlayVisible ? 'eye' : 'hidden'} />{t(overlayVisible ? '隐藏提示' : '显示提示')}</button>
      </div></div>
      <FramePreview capture={capture} calibrating={calibrating} onCalibrate={(rect) => { void calibrate(rect); }} onCancel={toggleCalibration} />
      <CoverReferencePicker
        reference={coverReference} selection={coverSelection}
        disabled={!native || calibrating || awaitingChange || refreshing}
        canSelect={capturing && Boolean(capture?.frame_url)} busy={busy !== ''}
        onBegin={() => { void beginCoverSelection(); }}
        onSave={(indices) => { void saveCoverSelection(indices); }}
        onCancel={() => { void cancelCoverSelection(); }}
        onClear={() => { void clearCoverReference(); }}
      />
      <div className="ba-preview-meta"><span>{capture?.width ? `${capture.width} × ${capture.height}` : '—'}<span className="ba-meta-divider">/</span>{t(capture?.board ? '棋盘已定位' : '等待定位棋盘')}</span><span className="ba-snapshot-meta">{t(manual ? '手动快照' : '自动更新')}<span className="ba-meta-divider">/</span>{refreshedTime}</span><span>{capture?.round !== null && capture?.round !== undefined ? t('第 {{round}} 轮', { round: capture.round }) : t('轮次 —')}<span className="ba-meta-divider">/</span>{t('剩余 {{value}}', { value: capture?.remaining != null && capture.remaining >= 0 ? capture.remaining : '—' })}</span></div>
    </section>

    <section className="ba-panel ba-board-panel" aria-labelledby="board-title">
      <div className="ba-panel-heading"><h2 id="board-title">{t('棋盘状态')} <span className="ba-heading-note">{BOARD_COLUMNS} × {BOARD_ROWS}</span></h2><div className="ba-inline-actions">
        {uncertainCount > 0 && <span className="ba-warning-tag">{t('{{count}} 格待确认', { count: uncertainCount })}</span>}
        <button className={`ba-button ba-button-small${emphasizeBest ? ' is-active' : ''}`} type="button" aria-pressed={emphasizeBest} title={t('用高对比粗边框和填色突出所有并列最高概率的未翻格')} onClick={toggleEmphasizeBest}>{t('突出最高')}</button>
        <button className="ba-button ba-button-small" type="button" disabled={!native || !capturing || editingDisabled} onClick={() => { void resetRound(); }}><Icon name="reset" />{t('重置轮次')}</button>
      </div></div>
      <div className="ba-correction-tools"><span>{t('点选校正为')}</span><div className="ba-brushes" role="group" aria-label={t('校正状态')}>{(Object.keys(cellLabels) as CellState[]).map((cell) => <button key={cell} className={`ba-brush ba-cell-${cell}${brush === cell ? ' is-selected' : ''}`} type="button" aria-pressed={brush === cell} onClick={() => setBrush(cell)}>{t(cellLabels[cell])}</button>)}</div></div>
      <div className="ba-cell-grid-wrap"><div className="ba-cell-grid" aria-label={t('9 列 5 行棋盘')}>{cells.map((cell, index) => <button key={index} type="button" className={`ba-cell ba-cell-${cell}${best.has(index) ? ' is-best' : ''}${emphasizeBest && best.has(index) ? ' is-emphasized' : ''}`} disabled={!native || !capturing || capture?.board == null || editingDisabled || calibrating} title={t('第 {{row}} 行，第 {{column}} 列：{{state}}', { row: Math.floor(index / BOARD_COLUMNS) + 1, column: index % BOARD_COLUMNS + 1, state: t(cellLabels[cell]) })} aria-label={t('第 {{row}} 行，第 {{column}} 列，{{state}}，校正为{{brush}}', { row: Math.floor(index / BOARD_COLUMNS) + 1, column: index % BOARD_COLUMNS + 1, state: t(cellLabels[cell]), brush: t(cellLabels[brush]) })} onClick={() => { void correctCell(index, brush); }}>
        {cell === 'unknown' && probabilities !== null ? <span className="ba-cell-probability">{percent(probabilities[index])}</span> : <span>{cell === 'empty' ? t('空') : cellSymbols[cell]}</span>}
      </button>)}</div><InferenceLayer placements={inferredPlacements} /></div>
      <div className={`ba-calculation-status${solverPhase === 'error' ? ' is-error' : ''}`} role="status"><i className={solverPhase === 'calculating' ? 'is-working' : ''} /><span>{translateDesktopMessage(calculationStatus, t)}</span>{solverPhase === 'ready' && result !== null && <span className="ba-calculation-detail">{result.precision === 'sampled' ? t('{{value}} 个样本', { value: result.samples.toLocaleString(locale) }) : t('{{value}} 种布局', { value: result.total_patterns })}</span>}</div>
    </section>

    <section className="ba-panel ba-items-panel" aria-labelledby="items-title">
      <div className="ba-panel-heading"><h2 id="items-title">{t('物品参数')}</h2><div className="ba-inline-actions"><span className={`ba-confirm-state${confirmed ? ' is-confirmed' : ''}`}>{t(confirmed ? '参数可用' : dirty ? '参数已修改' : '待读取')}</span><button className="ba-button ba-button-small ba-button-primary" type="button" disabled={!native || !capturing || editingDisabled || !validItems(items) || confirmed} onClick={() => { void confirmItems(); }}><Icon name="check" />{t('校正参数')}</button></div></div>
      <div className="ba-item-table"><div className="ba-item-table-head"><span>{t('概率筛选')}</span><span>{t('宽')}</span><span>{t('高')}</span><span title={t('尚未完整找到的物品件数，包含部分命中')}>{t('剩余件数')}</span></div>{items.map((item, index) => <div className="ba-item-row" key={index}>
        <button className={`ba-item-filter ba-item-${index}${filters[index] ? ' is-selected' : ''}`} type="button" aria-pressed={filters[index]} title={t('显示物品 {{item}} 的概率', { item: index + 1 })} onClick={() => toggleFilter(index)}><span className="ba-item-chip">{index + 1}</span><span>{t('物品 {{item}}', { item: index + 1 })}</span><span className="ba-filter-tick">{filters[index] ? '✓' : ''}</span></button>
        {(['width', 'height', 'remaining_count'] as const).map((field) => <input key={field} type="number" inputMode="numeric" min={field === 'remaining_count' ? 0 : 1} max={field === 'remaining_count' ? 7 : Math.max(BOARD_COLUMNS, BOARD_ROWS)} step={1} value={Number.isFinite(item[field]) && item[field] >= (field === 'remaining_count' ? 0 : 1) ? item[field] : ''} placeholder={t('待读取')} aria-label={t(field === 'width' ? '物品 {{item}} 的宽度' : field === 'height' ? '物品 {{item}} 的高度' : '物品 {{item}} 的剩余件数', { item: index + 1 })} disabled={editingDisabled} onChange={(event) => editItem(index, field, event.target.value === '' ? NaN : Number(event.target.value))} />)}
      </div>)}</div>
      {dirty && !validItems(items) && <p className="ba-item-validation" role="status">{t('尺寸需为正整数且旋转后能放入 9×5 棋盘；剩余件数为 0–7，总占格不能超过 45。')}</p>}
    </section>
    <footer className="ba-footer"><span>{t('提示当前筛选物品的命中概率')}</span><span>{t('局部命中时暂停 · 完整翻出后更新')}</span></footer>
  </main>;
}

export default function DesktopApp() {
  const params = new URLSearchParams(window.location.search);
  if (params.get('refresh') === '1') return <RefreshControl />;
  return params.get('overlay') === '1' ? <TransparentOverlay /> : <ControlWindow />;
}
