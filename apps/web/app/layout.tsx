import type { Metadata } from "next";
import type { ReactNode } from "react";

import { SERVER_THEME_BOOTSTRAP_SCRIPT } from "../platform/server-theme-cache";
import "../styles/product.css";
import "../styles/product-v2.css";

export const metadata: Metadata = {
  title: "rove",
  description: "Local-first agent product shell for workspaces, sessions, and runs.",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: ReactNode;
}>) {
  return (
    <html lang="en" data-theme="light" suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: SERVER_THEME_BOOTSTRAP_SCRIPT }} />
      </head>
      <body>{children}</body>
    </html>
  );
}
