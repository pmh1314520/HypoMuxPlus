import { motion } from "framer-motion";
import { SpeedHero } from "./SpeedHero";
import { AdapterTable } from "./AdapterTable";
import { MonitorPanel } from "./MonitorPanel";
import { LinkDistribution } from "./LinkDistribution";
import type { AdapterInfo, ConnInfo, NicTelemetry, TelemetryPayload } from "../lib/api";
import type { ClosedConn } from "../App";

interface Props {
  telemetry: TelemetryPayload | null;
  history: number[];
  peak: number;
  uptime: number;
  sessionMB: number;
  running: boolean;
  busy: boolean;
  canBoost: boolean;
  onBoost: () => void;

  adapters: AdapterInfo[];
  selected: Set<number>;
  toggle: (i: number) => void;
  selectAll: () => void;
  deselectAll: () => void;
  applySelection: (indices: number[]) => void;
  refresh: () => void;
  perNic: Record<string, NicTelemetry>;
  nicHistory: Record<string, number[]>;
  loading: boolean;

  logs: string[];
  clearLogs: () => void;
  connections: ConnInfo[];
  connHistory: ClosedConn[];
  clearHistory: () => void;
}

export function Dashboard(props: Props) {
  const container = { hidden: {}, show: { transition: { staggerChildren: 0.08 } } };
  const item = { hidden: { opacity: 0, y: 14 }, show: { opacity: 1, y: 0 } };

  return (
    <motion.div variants={container} initial="hidden" animate="show" className="h-full flex flex-col gap-4">
      <motion.div variants={item}>
        <SpeedHero
          telemetry={props.telemetry}
          history={props.history}
          peak={props.peak}
          uptime={props.uptime}
          sessionMB={props.sessionMB}
          running={props.running}
          busy={props.busy}
          canBoost={props.canBoost}
          onBoost={props.onBoost}
        />
      </motion.div>
      <motion.div variants={item} className="flex-1 grid gap-4 min-h-0" style={{ gridTemplateColumns: "1.45fr 1fr" }}>
        <AdapterTable
          adapters={props.adapters}
          selected={props.selected}
          toggle={props.toggle}
          selectAll={props.selectAll}
          deselectAll={props.deselectAll}
          applySelection={props.applySelection}
          refresh={props.refresh}
          perNic={props.perNic}
          nicHistory={props.nicHistory}
          running={props.running}
          loading={props.loading}
        />
        <div className="grid gap-4 min-h-0" style={{ gridTemplateRows: "1fr 1fr" }}>
          <LinkDistribution perNic={props.perNic} running={props.running} />
          <MonitorPanel
            logs={props.logs}
            clearLogs={props.clearLogs}
            connections={props.connections}
            connHistory={props.connHistory}
            clearHistory={props.clearHistory}
            running={props.running}
          />
        </div>
      </motion.div>
    </motion.div>
  );
}
