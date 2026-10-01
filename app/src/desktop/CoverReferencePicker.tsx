import { useId, useState } from 'react';
import { BOARD_COLUMNS, type CoverReferenceInfo, type CoverSelection } from './contracts.ts';
import { useDesktopI18n } from './desktopI18n.ts';
import './CoverReferencePicker.css';

interface CoverReferencePickerProps {
  reference: CoverReferenceInfo;
  selection: CoverSelection | null;
  disabled: boolean;
  busy: boolean;
  canSelect?: boolean;
  onBegin: () => void;
  onSave: (indices: number[]) => void;
  onCancel: () => void;
  onClear: () => void;
}

function SampleSelection({ selection, disabled, busy, onSave, onCancel }: {
  selection: CoverSelection;
  disabled: boolean;
  busy: boolean;
  onSave: (indices: number[]) => void;
  onCancel: () => void;
}) {
  const { t } = useDesktopI18n();
  const helpId = useId();
  const [indices, setIndices] = useState<number[]>([]);

  function toggle(index: number) {
    setIndices((previous) => previous.includes(index)
      ? previous.filter((selected) => selected !== index)
      : [...previous, index]);
  }

  return <div className="ba-cover-selection" onKeyDown={(event) => {
    if (event.key === 'Escape' && !disabled && !busy) {
      event.preventDefault();
      event.stopPropagation();
      onCancel();
    }
  }}>
    <div className="ba-cover-selection-heading">
      <strong>{t('选择未翻开样本')}</strong>
      <span aria-live="polite">{t('已选 {{count}} 格', { count: indices.length })}</span>
    </div>
    <p id={helpId} className="ba-cover-selection-help">{t('选择仍未翻开的方块，每种外观选一格即可。')}</p>
    <div
      className="ba-cover-selection-grid"
      role="group"
      aria-label={t('9 列 5 行棋盘')}
      aria-describedby={helpId}
      style={{ gridTemplateColumns: `repeat(${BOARD_COLUMNS}, minmax(0, 1fr))` }}
    >
      {selection.images.map((image, index) => {
        const selected = indices.includes(index);
        return <button
          className={`ba-cover-selection-cell${selected ? ' is-selected' : ''}`}
          key={index}
          type="button"
          aria-label={t('第 {{row}} 行，第 {{column}} 列', {
            row: Math.floor(index / BOARD_COLUMNS) + 1,
            column: index % BOARD_COLUMNS + 1,
          })}
          aria-pressed={selected}
          disabled={disabled || busy}
          onClick={() => toggle(index)}
        >
          <img src={image} alt="" draggable={false} />
          {selected && <span className="ba-cover-selection-check" aria-hidden="true">✓</span>}
        </button>;
      })}
    </div>
    <p className="ba-cover-selection-help">{t('保存后跨轮次、重启保留，识别异常不会自动替换。')}</p>
    <div className="ba-inline-actions">
      <button
        className="ba-button ba-button-small ba-button-primary"
        type="button"
        disabled={disabled || busy || indices.length === 0}
        onClick={() => onSave([...indices].sort((left, right) => left - right))}
      >{t('保存')}</button>
      <button className="ba-button ba-button-small" type="button" disabled={disabled || busy} onClick={onCancel}>{t('取消')}</button>
    </div>
  </div>;
}

export default function CoverReferencePicker({ reference, selection, disabled, busy, canSelect = true, onBegin, onSave, onCancel, onClear }: CoverReferencePickerProps) {
  const { t } = useDesktopI18n();
  const headingId = useId();
  const hasReference = reference.count > 0;
  const hasReferenceError = Boolean(reference.error);

  return <section className="ba-cover-reference" aria-labelledby={headingId} aria-busy={busy}>
    <div className="ba-cover-reference-summary">
      <strong id={headingId}>{t('未翻开样本')}</strong>
      <span className="ba-cover-reference-status">{t(hasReference ? '已固定 {{count}} 个样本' : '自动识别', { count: reference.count })}</span>
      {selection === null && <div className="ba-inline-actions">
        <button className="ba-button ba-button-small" type="button" disabled={disabled || busy || !canSelect} onClick={onBegin}>{t(hasReference ? '更新样本' : '选择样本')}</button>
        {(hasReference || hasReferenceError) && <button className="ba-button ba-button-small" type="button" disabled={disabled || busy} onClick={onClear}>{t('清除')}</button>}
      </div>}
    </div>
    {hasReferenceError && <p className="ba-cover-selection-help ba-cover-reference-error" role="alert">{t('未翻开样本文件无法读取，请清除或重新选择')}</p>}
    {hasReference && <div className="ba-cover-reference-thumbnails">
      {reference.images.map((image, index) => <img key={index} src={image} alt={t('未翻开样本 {{index}}', { index: index + 1 })} draggable={false} />)}
    </div>}
    {selection !== null && <SampleSelection key={selection.token} selection={selection} disabled={disabled} busy={busy} onSave={onSave} onCancel={onCancel} />}
  </section>;
}
