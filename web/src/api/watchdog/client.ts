// Watchdog: anomalies detected over the past day without monitors (APM error rate and latency, error
// logs, host CPU, memory and disk). Outside live mode the in-browser mock (mock/watchdog.ts) answers.
//   GET /api/v1/watchdog/alerts   watchdogAlerts
import { API_MODE, get } from '../http';
import type { WatchdogAlerts } from '../../lib/watchdog';
import * as mock from '../mock/watchdog';

export type { WatchdogAlert, WatchdogAlerts, WatchdogPoint } from '../../lib/watchdog';

export const watchdogAlerts = (signal?: AbortSignal): Promise<WatchdogAlerts> => (API_MODE === 'live' ? get('/api/v1/watchdog/alerts', {}, signal) : mock.watchdogAlerts());
