import { proxyRoveApiRequest } from "../../../lib/rove-api-proxy";

type RouteContext = {
  params: Promise<{
    path?: string[];
  }>;
};

async function handle(request: Request, context: RouteContext): Promise<Response> {
  const params = await context.params;
  return proxyRoveApiRequest(request, params.path ?? []);
}

export async function GET(
  request: Request,
  context: RouteContext,
): Promise<Response> {
  return handle(request, context);
}

export async function POST(
  request: Request,
  context: RouteContext,
): Promise<Response> {
  return handle(request, context);
}

export async function PUT(
  request: Request,
  context: RouteContext,
): Promise<Response> {
  return handle(request, context);
}

export async function PATCH(
  request: Request,
  context: RouteContext,
): Promise<Response> {
  return handle(request, context);
}

export async function DELETE(
  request: Request,
  context: RouteContext,
): Promise<Response> {
  return handle(request, context);
}
