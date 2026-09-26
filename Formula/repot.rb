class Repot < Formula
  desc "Keep Git repositories organised, current and portable"
  homepage "https://github.com/niklas-heer/repot"
  head "https://github.com/niklas-heer/repot.git", branch: "main"
  license "MIT"

  depends_on "rust" => :build
  depends_on "git"

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "repot ", shell_output("#{bin}/repot --version")
    ENV["HOME"] = testpath
    ENV["GHQ_ROOT"] = testpath/"repos"
    ENV["XDG_CONFIG_HOME"] = testpath/"config"
    mkdir "repos"
    assert_equal "[]", shell_output("#{bin}/repot list --json").strip
  end
end
