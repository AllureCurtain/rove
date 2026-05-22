export type RunStatus = "init" | "running" | "done" | "error" | "cancelled";

export interface Usage {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
}

export interface PlanStep {
  id: string;
  title: string;
  done: boolean;
}

export interface TaskPlan {
  goal: string;
  steps: PlanStep[];
  current_step: number;
}

export interface ToolResult {
  call_id: string;
  output: string;
}

export interface ToolError {
  code: string;
  reason?: string;
  timeout_ms?: number;
  name?: string;
  [key: string]: unknown;
}

export type StreamEvent =
  | {
      type: "run_started";
      run_id: string;
      job_id: string;
      user_message: string;
    }
  | {
      type: "llm_chunk";
      delta: string;
    }
  | {
      type: "llm_message";
      full: string;
      usage: Usage;
    }
  | {
      type: "tool_call_started";
      call_id: string;
      name: string;
      args: unknown;
    }
  | {
      type: "tool_call_completed";
      call_id: string;
      result: ToolResult;
    }
  | {
      type: "tool_call_failed";
      call_id: string;
      error: ToolError;
    }
  | {
      type: "plan_created";
      plan: TaskPlan;
    }
  | {
      type: "plan_step_started";
      step: PlanStep;
      index: number;
    }
  | {
      type: "plan_step_completed";
      step: PlanStep;
      index: number;
    }
  | {
      type: "run_completed";
      reason: string;
      output?: string | null;
    };

export interface CreateJobRequest {
  message: string;
  model?: string;
  max_steps?: number;
}

export interface CreateJobResponse {
  job_id: string;
  run_id: string;
}

export interface JobStateResponse {
  job_id: string;
  run_id: string;
  status: RunStatus;
  event_count: number;
}

export const STREAM_EVENT_NAMES = [
  "run_started",
  "llm_chunk",
  "llm_message",
  "tool_call_started",
  "tool_call_completed",
  "tool_call_failed",
  "plan_created",
  "plan_step_started",
  "plan_step_completed",
  "run_completed",
] as const;

export type StreamEventName = (typeof STREAM_EVENT_NAMES)[number];
