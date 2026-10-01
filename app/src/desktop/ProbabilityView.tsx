import { BOARD_SIZE, type InferredPlacement, type OverlayState } from './contracts.ts';
import { bestCells, percent } from './logic.ts';
import { translateDesktopMessage, useDesktopI18n } from './desktopI18n.ts';

export function InferenceLayer({ placements }: { placements: InferredPlacement[] }) {
  const { t } = useDesktopI18n();
  if (placements.length === 0) return null;
  return <div className="ba-inference-layer" aria-label={t('可靠物品位置推断')}>
    {placements.map((placement) => <div
      className="ba-inferred-placement"
      key={`${placement.item_index}:${placement.x}:${placement.y}:${placement.width}:${placement.height}`}
      data-inferred-item={placement.item_index}
      title={t('物品 {{item}} 的可靠位置推断', { item: placement.item_index + 1 })}
      style={{
        gridColumn: `${placement.x + 1} / span ${placement.width}`,
        gridRow: `${placement.y + 1} / span ${placement.height}`,
      }}
    ><span>{t('推断')}</span></div>)}
  </div>;
}

export function OverlayView({ overlay }: { overlay: OverlayState }) {
  const { t } = useDesktopI18n();
  const best = bestCells(overlay.probabilities, overlay.cells);
  const complete = overlay.probabilities.length === BOARD_SIZE && overlay.cells.length === BOARD_SIZE;
  return <main className="ba-overlay" aria-label={t('棋盘概率提示')}>
    {overlay.visible && <>
      {complete && <>
        <div className="ba-overlay-grid">{overlay.cells.map((cell, index) => <div key={index} className={`ba-overlay-cell${cell === 'unknown' && best.has(index) ? ' is-best' : ''}${overlay.emphasize_best && best.has(index) ? ' is-emphasized' : ''}`}>
          {cell === 'unknown' && Number.isFinite(overlay.probabilities[index]) && <span>{percent(overlay.probabilities[index])}</span>}
        </div>)}</div>
        <InferenceLayer placements={overlay.inferred_placements} />
      </>}
      {overlay.message !== '' && <span className="ba-overlay-status">{translateDesktopMessage(overlay.message, t)}{overlay.precision === 'sampled' && !overlay.message.includes('估计') ? ` · ${t('估计')}` : ''}</span>}
    </>}
  </main>;
}

