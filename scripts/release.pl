#!/usr/bin/env perl

use strict;
use warnings;

use Cwd qw(abs_path);
use Digest::SHA ();
use File::Basename qw(dirname);
use File::Copy qw(copy);
use File::Find qw(find);
use File::Path qw(make_path);
use File::Spec;
use File::Temp qw(tempdir);
use FindBin qw($Bin);
use Getopt::Long qw(GetOptions);
use JSON::PP qw(decode_json);

my $root = abs_path(File::Spec->catdir($Bin, '..'));
my $target;
my $arch;

GetOptions(
    'target=s' => \$target,
    'arch=s'   => \$arch,
) or die usage();

@ARGV == 0 or die usage();
chdir $root or die "failed to enter $root: $!\n";

my $versions = read_versions(File::Spec->catfile($root, 'version'));
verify_cargo_versions($versions);

$arch //= architecture_name($target);
validate_fragment('architecture', $arch);

my $build_target = File::Spec->catdir($root, 'target', 'release-build');
build_binaries($build_target, $target);

my $binary_directory = defined $target
    ? File::Spec->catdir($build_target, $target, 'release')
    : File::Spec->catdir($build_target, 'release');

my @archives;
push @archives, package_binary(
    product => 'kome',
    version => $versions->{'kome-sdk'},
    arch => $arch,
    binary => File::Spec->catfile($binary_directory, executable_name('kome')),
);
push @archives, package_binary(
    product => 'komec',
    version => $versions->{'kome-sdk'},
    arch => $arch,
    binary => File::Spec->catfile($binary_directory, executable_name('komec')),
    support_files => [
        File::Spec->catfile($binary_directory, executable_name('kome-lsp')),
        File::Spec->catfile($binary_directory, 'libkome_native_rt.a'),
    ],
);
push @archives, package_stdlib(
    version => $versions->{'kome-sdk'},
    arch => $arch,
    source => File::Spec->catdir($root, 'vendor', 'stdlib'),
);
write_checksum_manifest(@archives);

sub usage {
    return "usage: scripts/release.pl [--target <rust-target>] [--arch <artifact-arch>]\n";
}

sub read_versions {
    my ($path) = @_;
    open my $file, '<', $path or die "failed to read $path: $!\n";

    my %versions;
    my $line_number = 0;
    while (my $line = <$file>) {
        ++$line_number;
        chomp $line;
        $line =~ s/\r\z//;
        next if $line =~ /^\s*(?:#|\z)/;

        $line =~ /\A(kome-sdk|kome|komec|kome-std)=([^\s=]+)\z/
            or die "invalid version entry at $path:$line_number\n";
        exists $versions{$1}
            and die "duplicate version entry for $1 at $path:$line_number\n";
        validate_fragment("version for $1", $2);
        $versions{$1} = $2;
    }
    close $file or die "failed to close $path: $!\n";

    for my $product (qw(kome-sdk kome komec kome-std)) {
        exists $versions{$product}
            or die "missing $product version in $path\n";
    }
    return \%versions;
}

sub verify_cargo_versions {
    my ($versions) = @_;
    open my $metadata, '-|', 'cargo', 'metadata', '--format-version', '1', '--no-deps'
        or die "failed to start cargo metadata: $!\n";
    local $/;
    my $json = <$metadata>;
    close $metadata or die "cargo metadata failed\n";

    my $decoded = decode_json($json);
    my %cargo_versions = map { $_->{name} => $_->{version} }
        grep { $_->{name} eq 'kome' || $_->{name} eq 'komec' }
        @{$decoded->{packages}};

    for my $product (qw(kome komec)) {
        defined $cargo_versions{$product}
            or die "cargo package $product was not found\n";
        $cargo_versions{$product} eq $versions->{$product}
            or die "$product version mismatch: version file has $versions->{$product}, "
                . "Cargo.toml has $cargo_versions{$product}\n";
    }
}

sub architecture_name {
    my ($target) = @_;
    return (split /-/, $target, 2)[0] if defined $target;

    open my $rustc, '-|', 'rustc', '-vV' or die "failed to start rustc: $!\n";
    my $host;
    while (my $line = <$rustc>) {
        if ($line =~ /^host:\s+([^\s]+)/) {
            $host = $1;
        }
    }
    close $rustc or die "rustc -vV failed\n";
    defined $host or die "rustc did not report a host architecture\n";
    return (split /-/, $host, 2)[0];
}

sub validate_fragment {
    my ($description, $value) = @_;
    $value =~ /\A[A-Za-z0-9][A-Za-z0-9._+-]*\z/
        or die "invalid $description: $value\n";
}

sub build_binaries {
    my ($build_target, $target) = @_;
    make_path($build_target);

    local $ENV{CARGO_TARGET_DIR} = $build_target;
    my @command = (
        'cargo', 'build', '--release', '--locked',
        '-p', 'kome', '--bin', 'kome',
        '-p', 'komec', '--bin', 'komec',
        '-p', 'kome_lsp', '--bin', 'kome-lsp',
    );
    push @command, '--target', $target if defined $target;
    run(@command);

    my @runtime_command = (
        'cargo', 'build', '--release', '--locked',
        '-p', 'kome_native_rt', '--lib',
    );
    push @runtime_command, '--target', $target if defined $target;
    run(@runtime_command);
}

sub executable_name {
    my ($name) = @_;
    return $^O eq 'MSWin32' ? "$name.exe" : $name;
}

sub package_binary {
    my (%arguments) = @_;
    -f $arguments{binary}
        or die "release binary was not found: $arguments{binary}\n";

    my $stage = tempdir('kome-release-XXXXXX', TMPDIR => 1, CLEANUP => 1);
    my $staged_binary = File::Spec->catfile(
        $stage,
        executable_name($arguments{product}),
    );
    copy($arguments{binary}, $staged_binary)
        or die "failed to stage $arguments{binary}: $!\n";
    chmod 0755, $staged_binary
        or die "failed to mark $staged_binary executable: $!\n";

    my @entries = (executable_name($arguments{product}));
    for my $support_file (@{$arguments{support_files} // []}) {
        -f $support_file
            or die "release support file was not found: $support_file\n";
        my $name = basename_of($support_file);
        copy($support_file, File::Spec->catfile($stage, $name))
            or die "failed to stage $support_file: $!\n";
        push @entries, $name;
    }

    return create_artifact(
        product => $arguments{product},
        version => $arguments{version},
        arch => $arguments{arch},
        stage => $stage,
        entries => \@entries,
    );
}

sub package_stdlib {
    my (%arguments) = @_;
    -d $arguments{source}
        or die "standard library was not found: $arguments{source}\n";

    my $stage = tempdir('kome-release-XXXXXX', TMPDIR => 1, CLEANUP => 1);
    my @entries;
    find(
        {
            no_chdir => 1,
            wanted => sub {
                return unless -f $_ && $_ =~ /\.kome\z/;
                my $relative = File::Spec->abs2rel($_, $arguments{source});
                my $destination = File::Spec->catfile($stage, $relative);
                make_path(dirname($destination));
                copy($_, $destination)
                    or die "failed to stage $_: $!\n";
                push @entries, $relative;
            },
        },
        $arguments{source},
    );
    @entries = sort @entries;
    @entries or die "standard library contains no .kome files\n";

    return create_artifact(
        product => 'kome-std',
        version => $arguments{version},
        arch => $arguments{arch},
        stage => $stage,
        entries => \@entries,
    );
}

sub create_artifact {
    my (%arguments) = @_;
    my $output_directory = File::Spec->catdir($root, 'target', 'release');
    make_path($output_directory);

    my $filename = join '-',
        $arguments{arch},
        $arguments{product},
        $arguments{version};
    my $archive = File::Spec->catfile($output_directory, "$filename.tar.zst");
    my $tar = File::Spec->catfile($arguments{stage}, "$filename.tar");
    my $epoch = source_date_epoch();

    run(
        'tar',
        '--sort=name',
        "--mtime=\@$epoch",
        '--owner=0',
        '--group=0',
        '--numeric-owner',
        '-C', $arguments{stage},
        '-cf', $tar,
        @{$arguments{entries}},
    );
    run('zstd', '-q', '-19', '-f', $tar, '-o', $archive);
    unlink $tar or die "failed to remove temporary archive $tar: $!\n";

    print "$archive\n";
    return $archive;
}

sub write_checksum_manifest {
    my (@archives) = @_;
    my $output_directory = File::Spec->catdir($root, 'target', 'release');
    my $checksum_path = File::Spec->catfile($output_directory, 'SHA256SUMS');
    my $temporary = "$checksum_path.tmp.$$";

    open my $checksum, '>', $temporary
        or die "failed to write $temporary: $!\n";
    for my $archive (@archives) {
        open my $input, '<', $archive or die "failed to read $archive: $!\n";
        binmode $input;
        my $digest = Digest::SHA->new(256)->addfile($input)->hexdigest;
        close $input or die "failed to close $archive: $!\n";
        print {$checksum} "$digest  " . basename_of($archive) . "\n";
    }
    close $checksum or die "failed to close $temporary: $!\n";
    rename $temporary, $checksum_path
        or die "failed to replace $checksum_path: $!\n";
    print "$checksum_path\n";
}

sub source_date_epoch {
    return $ENV{SOURCE_DATE_EPOCH}
        if defined $ENV{SOURCE_DATE_EPOCH} && $ENV{SOURCE_DATE_EPOCH} =~ /\A\d+\z/;

    open my $git, '-|', 'git', 'log', '-1', '--format=%ct'
        or die "failed to start git log: $!\n";
    my $epoch = <$git>;
    close $git or die "git log failed\n";
    chomp $epoch;
    $epoch =~ /\A\d+\z/ or die "git log returned an invalid timestamp\n";
    return $epoch;
}

sub basename_of {
    my ($path) = @_;
    $path =~ s{.*[\\/]}{};
    return $path;
}

sub run {
    my (@command) = @_;
    print '+ ', join(' ', @command), "\n";
    system @command;
    if ($? == -1) {
        die "failed to execute $command[0]: $!\n";
    }
    if ($? & 127) {
        die "$command[0] terminated by signal " . ($? & 127) . "\n";
    }
    my $status = $? >> 8;
    $status == 0 or die "$command[0] exited with status $status\n";
}
