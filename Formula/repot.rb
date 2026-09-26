class Repot < Formula
  desc "Keep Git repositories organised, current and portable"
  homepage "https://github.com/niklas-heer/repot"
  head "https://github.com/niklas-heer/repot.git", branch: "main"
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/niklas-heer/repot/releases/download/v0.1.0/repot-0.1.0-aarch64-apple-darwin.tar.gz"
      sha256 "1f71ba1d54abd1bb949cad2bbf724ea7a2e86c1a311e0b299d833e05a9105600"
    end
    on_intel do
      url "https://github.com/niklas-heer/repot/releases/download/v0.1.0/repot-0.1.0-x86_64-apple-darwin.tar.gz"
      sha256 "46bce0acc7a4fe900b10c5a8d2978e78a97c60d7c5097586a2a32bc4bdefe978"
    end
  end
  on_linux do
    on_arm do
      url "https://github.com/niklas-heer/repot/releases/download/v0.1.0/repot-0.1.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "457dc634f105aa98ac47bc339eb9134912080f7299af6440d2beef8e68757fd9"
    end
    on_intel do
      url "https://github.com/niklas-heer/repot/releases/download/v0.1.0/repot-0.1.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "f5d4c95e8a8fff02c45c9ffcf54a91c7dda7348df1fbeb1fa2fbc833cd13939f"
    end
  end

  depends_on "git"
  depends_on "rust" => :build if build.head?

  def install
    if build.head?
      system "cargo", "install", *std_cargo_args
    else
      bin.install "repot"
    end
  end

  test do
    expected = build.head? ? "repot " : "repot #{version}"
    assert_match expected, shell_output("#{bin}/repot --version")
    ENV["HOME"] = testpath
    ENV["GHQ_ROOT"] = testpath/"repos"
    ENV["XDG_CONFIG_HOME"] = testpath/"config"
    mkdir "repos"
    assert_equal "[]", shell_output("#{bin}/repot list --json").strip
  end
end
