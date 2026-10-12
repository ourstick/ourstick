// Vite turns an imported picture into its URL.
declare module "*.png" {
  const url: string;
  export default url;
}
