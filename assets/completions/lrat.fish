# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_lrat_global_optspecs
    string join \n demo h/help V/version
end

function __fish_lrat_needs_command
    # Figure out if the current invocation already has a command.
    set -l cmd (commandline -opc)
    set -e cmd[1]
    argparse -s (__fish_lrat_global_optspecs) -- $cmd 2>/dev/null
    or return
    if set -q argv[1]
        # Also print the command, so this can be used to figure out what it is.
        echo $argv[1]
        return 1
    end
    return 0
end

function __fish_lrat_using_subcommand
    set -l cmd (__fish_lrat_needs_command)
    test -z "$cmd"
    and return 1
    contains -- $cmd[1] $argv
end

complete -c lrat -n "__fish_lrat_needs_command" -l demo -d 'Run without a Linear account or API key'
complete -c lrat -n "__fish_lrat_needs_command" -s h -l help -d 'Print help'
complete -c lrat -n "__fish_lrat_needs_command" -s V -l version -d 'Print version'
complete -c lrat -n "__fish_lrat_needs_command" -f -a "auth" -d 'Manage the Linear API key'
complete -c lrat -n "__fish_lrat_needs_command" -f -a "completions" -d 'Print the completion script for a shell'
complete -c lrat -n "__fish_lrat_needs_command" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c lrat -n "__fish_lrat_using_subcommand auth; and not __fish_seen_subcommand_from login status logout help" -s h -l help -d 'Print help'
complete -c lrat -n "__fish_lrat_using_subcommand auth; and not __fish_seen_subcommand_from login status logout help" -f -a "login" -d 'Check an API key with Linear and store it in the OS keyring'
complete -c lrat -n "__fish_lrat_using_subcommand auth; and not __fish_seen_subcommand_from login status logout help" -f -a "status" -d 'Show where the API key comes from, and its workspace and viewer'
complete -c lrat -n "__fish_lrat_using_subcommand auth; and not __fish_seen_subcommand_from login status logout help" -f -a "logout" -d 'Delete the API key from the OS keyring'
complete -c lrat -n "__fish_lrat_using_subcommand auth; and not __fish_seen_subcommand_from login status logout help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c lrat -n "__fish_lrat_using_subcommand auth; and __fish_seen_subcommand_from login" -l from-keyring -d 'Read the key from an existing OS keyring entry instead of prompting for it' -r
complete -c lrat -n "__fish_lrat_using_subcommand auth; and __fish_seen_subcommand_from login" -l stdin -d 'Read the key from standard input instead of prompting for it'
complete -c lrat -n "__fish_lrat_using_subcommand auth; and __fish_seen_subcommand_from login" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c lrat -n "__fish_lrat_using_subcommand auth; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c lrat -n "__fish_lrat_using_subcommand auth; and __fish_seen_subcommand_from logout" -s h -l help -d 'Print help'
complete -c lrat -n "__fish_lrat_using_subcommand auth; and __fish_seen_subcommand_from help" -f -a "login" -d 'Check an API key with Linear and store it in the OS keyring'
complete -c lrat -n "__fish_lrat_using_subcommand auth; and __fish_seen_subcommand_from help" -f -a "status" -d 'Show where the API key comes from, and its workspace and viewer'
complete -c lrat -n "__fish_lrat_using_subcommand auth; and __fish_seen_subcommand_from help" -f -a "logout" -d 'Delete the API key from the OS keyring'
complete -c lrat -n "__fish_lrat_using_subcommand auth; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c lrat -n "__fish_lrat_using_subcommand completions" -s h -l help -d 'Print help'
complete -c lrat -n "__fish_lrat_using_subcommand help; and not __fish_seen_subcommand_from auth completions help" -f -a "auth" -d 'Manage the Linear API key'
complete -c lrat -n "__fish_lrat_using_subcommand help; and not __fish_seen_subcommand_from auth completions help" -f -a "completions" -d 'Print the completion script for a shell'
complete -c lrat -n "__fish_lrat_using_subcommand help; and not __fish_seen_subcommand_from auth completions help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c lrat -n "__fish_lrat_using_subcommand help; and __fish_seen_subcommand_from auth" -f -a "login" -d 'Check an API key with Linear and store it in the OS keyring'
complete -c lrat -n "__fish_lrat_using_subcommand help; and __fish_seen_subcommand_from auth" -f -a "status" -d 'Show where the API key comes from, and its workspace and viewer'
complete -c lrat -n "__fish_lrat_using_subcommand help; and __fish_seen_subcommand_from auth" -f -a "logout" -d 'Delete the API key from the OS keyring'
