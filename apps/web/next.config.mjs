/** @type {import('next').NextConfig} */
const nextConfig = {
  allowedDevOrigins: ["127.0.0.1"],
  // Expose only this validated, non-secret build-time preference to the client.
  env: {
    ROVE_DEFAULT_LOCALE:
      process.env.ROVE_DEFAULT_LOCALE === "en-US" ? "en-US" : "zh-CN",
  },
};

export default nextConfig;
