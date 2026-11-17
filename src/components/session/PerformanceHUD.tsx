import { motion, AnimatePresence } from 'framer-motion';
import { Activity, Gauge, MonitorDot, Wifi } from 'lucide-react';
import { useSessionStore } from '../../stores/sessionStore';
import { Card } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { cn } from '@/lib/utils';
import { formatBandwidth, fpsLevel, latencyLevel } from '../../lib/hud-format';
import type { GraphicsObservation, NetworkMetrics } from '../../types';

interface Props {
  fps: number;
  latency: number;
  network?: NetworkMetrics | null;
  graphics?: GraphicsObservation;
  resolution: string;
}

export function PerformanceHUD({ fps, latency, network, graphics, resolution }: Props) {
  const show = useSessionStore((s) => s.showPerformanceHud);

  const fpsTrend = fpsLevel(fps);

  const latencyTrend = latencyLevel(latency);

  return (
    <AnimatePresence>
      {show && (
        <motion.div
          initial={{ opacity: 0, x: 20 }}
          animate={{ opacity: 1, x: 0 }}
          exit={{ opacity: 0, x: 20 }}
          transition={{ duration: 0.2 }}
          className="absolute top-3 right-3 z-20 w-52"
        >
          <Card
            className="overflow-hidden border-0 gap-0 py-0"
            style={{
              background: 'rgba(0, 0, 0, 0.75)',
              backdropFilter: 'blur(12px)',
              border: '1px solid rgba(255,255,255,0.08)',
            }}
          >
            <div className="px-3 py-2 border-b border-white/5">
              <span className="text-[10px] font-semibold text-white/60 uppercase tracking-wider">
                Performance
              </span>
            </div>
            <div className="p-3 space-y-2.5">
              <HUDRow
                icon={Activity}
                label="FPS"
                value={String(fps)}
                level={fpsTrend}
              />
              <HUDRow
                icon={Wifi}
                label="Latency"
                value={`${latency}ms`}
                level={latencyTrend}
              />
              <HUDRow
                icon={Gauge}
                label="Receive"
                value={network ? formatBandwidth(network.rxBytesPerSecond) : 'Unknown'}
                level="info"
              />
              <HUDRow
                icon={Gauge}
                label="Send"
                value={network ? formatBandwidth(network.txBytesPerSecond) : 'Unknown'}
                level="info"
              />
              <HUDRow
                icon={MonitorDot}
                label="Resolution"
                value={resolution}
                level="neutral"
              />
              <div className="text-[10px] text-white/60 break-words" title="Observed on this connection, not requested capabilities. Unknown means the wire encoding is not identified.">
                <div>Channel: {graphics?.channels.join(', ') || 'Unknown'}</div>
                <div>Codecs: {graphics?.codecs.join(', ') || 'Unknown'}</div>
              </div>
              <div className="text-[9px] text-white/50" title="Includes RDP negotiation and TLS overhead. Excludes TCP/IP headers, retransmissions and separate authentication sockets. Totals reset on reconnect.">
                TCP payload · includes TLS
                {network && <div>Received {formatBandwidth(network.rxBytes).replace('/s', '')} · Sent {formatBandwidth(network.txBytes).replace('/s', '')}</div>}
              </div>
            </div>
          </Card>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

type HUDLevel = 'good' | 'medium' | 'bad' | 'info' | 'neutral';

const levelColors: Record<HUDLevel, string> = {
  good: 'bg-green-500/20 text-green-400 border-green-500/30',
  medium: 'bg-yellow-500/20 text-yellow-400 border-yellow-500/30',
  bad: 'bg-red-500/20 text-red-400 border-red-500/30',
  info: 'bg-indigo-500/20 text-indigo-400 border-indigo-500/30',
  neutral: 'bg-white/10 text-white/70 border-white/10',
};

const levelIconColors: Record<HUDLevel, string> = {
  good: 'text-green-400',
  medium: 'text-yellow-400',
  bad: 'text-red-400',
  info: 'text-indigo-400',
  neutral: 'text-white/60',
};

function HUDRow({
  icon: Icon,
  label,
  value,
  level,
}: {
  icon: React.ComponentType<{ size: number; className?: string }>;
  label: string;
  value: string;
  level: HUDLevel;
}) {
  return (
    <div className="flex items-center justify-between">
      <div className="flex items-center gap-1.5">
        <Icon size={11} className={levelIconColors[level]} />
        <span className="text-[11px] text-white/50">{label}</span>
      </div>
      <Badge
        variant="outline"
        className={cn(
          'text-[10px] font-mono font-medium h-4 px-1.5',
          levelColors[level]
        )}
      >
        {value}
      </Badge>
    </div>
  );
}
