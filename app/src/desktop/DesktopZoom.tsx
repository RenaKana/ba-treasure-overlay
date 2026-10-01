import { useEffect, useLayoutEffect, useState } from 'react';
import { useDesktopI18n } from './desktopI18n.ts';
import {
  CONTROL_ZOOM_LEVELS,
  CONTROL_ZOOM_STORAGE_KEY,
  DEFAULT_CONTROL_ZOOM,
  normalizeControlZoom,
  stepControlZoom,
} from './zoomPreferences.ts';

function storedZoom(): number {
  try {
    return normalizeControlZoom(localStorage.getItem(CONTROL_ZOOM_STORAGE_KEY));
  } catch {
    return DEFAULT_CONTROL_ZOOM;
  }
}

export default function DesktopZoom() {
  const { t } = useDesktopI18n();
  const [zoom, setZoom] = useState(storedZoom);

  useLayoutEffect(() => {
    document.documentElement.style.setProperty('--ba-ui-scale', String(zoom));
    try {
      localStorage.setItem(CONTROL_ZOOM_STORAGE_KEY, String(zoom));
    } catch {
      // The current window remains usable when preference storage is unavailable.
    }
    return () => { document.documentElement.style.removeProperty('--ba-ui-scale'); };
  }, [zoom]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((!event.ctrlKey && !event.metaKey) || event.altKey) return;
      if (event.key === '0') {
        event.preventDefault();
        setZoom(DEFAULT_CONTROL_ZOOM);
      } else if (event.key === '+' || event.key === '=') {
        event.preventDefault();
        setZoom((previous) => stepControlZoom(previous, 1));
      } else if (event.key === '-' || event.key === '_') {
        event.preventDefault();
        setZoom((previous) => stepControlZoom(previous, -1));
      }
    };
    const onWheel = (event: WheelEvent) => {
      if (!event.ctrlKey || event.deltaY === 0) return;
      event.preventDefault();
      setZoom((previous) => stepControlZoom(previous, event.deltaY < 0 ? 1 : -1));
    };
    window.addEventListener('keydown', onKeyDown, true);
    window.addEventListener('wheel', onWheel, { passive: false });
    return () => {
      window.removeEventListener('keydown', onKeyDown, true);
      window.removeEventListener('wheel', onWheel);
    };
  }, []);

  return <select
    id="desktop-zoom"
    aria-label={t('界面缩放')}
    title={t('界面缩放（Ctrl +/-，Ctrl 0 重置）')}
    value={zoom}
    onChange={(event) => setZoom(normalizeControlZoom(event.target.value))}
  >
    {CONTROL_ZOOM_LEVELS.map((level) => <option key={level} value={level}>{Math.round(level * 100)}%</option>)}
  </select>;
}
